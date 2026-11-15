//! Decompiling a compiled graph (decoded by [`crate::decode::graph_package::compiled`]) into an engine graph
//! package: one texture graph with an `input_value` per graph parameter (exposed under the parameter's
//! identifier), an `input` per image input, one engine node per atomic node, function subgraphs for
//! the parameter expressions ([`functions`]) and pattern programs for fx-maps ([`programs`]).
//!
//! This is the translate stage: the mapping from the compiled operations' conventions onto our
//! nodes lives here only. What does not translate is reported in [`Decompiled::warnings`] and replaced by a
//! neutral node, so the graph always loads.
// File bytes and compiled programs come from the user's files: no unchecked indexing here.
#![deny(clippy::indexing_slicing)]

mod arms;
pub mod bitmaps;
pub mod functions;
pub mod programs;
mod scalar;
mod usertag;

use crate::decode::graph_package::compiled::{
    Assembly, AtomicNode, BlendAlpha, Kind, NormalOptions, Operands, SampleDepth, TransformSampling, Value, ValueType,
};
use crate::decode::graph_package::{Description, GraphInputInfo, INPUT_TYPE_IMAGE};
use functions::{Functions, ValueNode};
use std::fmt::Write;
use std::sync::Arc;

pub use usertag::{default_color, working_space};

/// The version of the graph text written here: the graph runtime's format 3 (named codes and ops). The runtime reads
/// this number from the text's `format` line; it is ours, not the compiled format's.
pub const GRAPH_FORMAT: u32 = 3;

/// Node id of atomic node `i`.
fn nid(i: usize) -> usize {
    i + 1
}

/// Format data: a blend's cropping corners are u16 fixed point with 15 fraction bits (1.15), so 1.0 is this.
const CROP_ONE: f32 = 32768.0;

/// Format data: a transform without a stored sampling word reads pyramid level 31 (none), bilinear.
const TRANSFORM_DEFAULT_MIP: u32 = 31;

/// Ids of the parameter nodes start here.
const PARAM_IDS: usize = 100_000;

/// A black image, for image inputs a node leaves unconnected (stored as index -1).
const BLACK: usize = PARAM_IDS - 1;

/// Node id an image input refers to.
fn image_ref(i: usize) -> usize {
    if i == u32::MAX as usize { BLACK } else { nid(i) }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decompiled {
    /// The package as engine graph text, shared: it carries the embedded images and can be hundreds of MB.
    pub text: Arc<str>,
    /// Image inputs in `input idx` order: (identifier, label).
    pub images: Vec<String>,
    /// For each image input, the colour its `usertag` asks for when nothing feeds it.
    pub image_defaults: Vec<Option<[f32; 3]>>,
    /// Outputs: (identifier, node id).
    pub outputs: Vec<(String, usize)>,
    pub warnings: Vec<String>,
    /// The outputs whose node holds colour (the others are grey), from the compiled node.
    pub colour_outputs: Vec<String>,
}

/// Graph text for `asm` described by `desc`, rendered at `size` pixels.
pub fn decompile(desc: &Description, asm: &Assembly, size: usize) -> Decompiled {
    let mut fns = Functions::default();
    fns.values = value_nodes(asm);
    let mut warnings = Vec::new();
    let mut body = String::new();
    let mut programs = Vec::new();
    let mut embedded = Vec::new();

    // parameters (not images, not the $ system inputs)
    let params: Vec<&GraphInputInfo> = desc.inputs.iter().filter(|i| i.ty != INPUT_TYPE_IMAGE && !i.identifier.starts_with('$')).collect();
    for (k, p) in params.iter().enumerate() {
        let _ = writeln!(body, "node {} {}", PARAM_IDS + k, input_value(p));
    }
    let images: Vec<String> = desc.inputs.iter().filter(|i| i.ty == INPUT_TYPE_IMAGE).map(|i| i.identifier.clone()).collect();
    let image_defaults = desc.inputs.iter().filter(|i| i.ty == INPUT_TYPE_IMAGE).map(|i| i.usertag.as_deref().and_then(usertag::default_color)).collect();

    let mut colour_nodes = Vec::new();
    for (i, n) in asm.atomic_nodes().into_iter().enumerate() {
        colour_nodes.push(n.as_ref().is_ok_and(|a| a.color));
        let line = match n {
            Ok(a) => node_line(&a, desc, asm, &images, &mut fns, &mut programs, &mut embedded, &mut warnings),
            Err(e) => {
                warnings.push(format!("node {i}: {e}"));
                "color r=0 g=0 b=0".into()
            }
        };
        let _ = writeln!(body, "node {} {line}", nid(i));
    }

    if body.contains(&format!("=@{BLACK}")) {
        let _ = writeln!(body, "node {BLACK} color r=0 g=0 b=0");
    }

    let mut outputs = Vec::new();
    let mut colour_outputs = Vec::new();
    let s = asm.section();
    // output bindings: (format, node index) per output, right after the node table
    for (k, uid) in asm.outputs.iter().enumerate() {
        let Some(name) = desc.output(*uid).map(|o| o.identifier.clone()) else { continue };
        if let Some(node) = output_node(&s, asm, k) {
            if colour_nodes.get(node).copied().unwrap_or(false) {
                colour_outputs.push(name.clone());
            }
            outputs.push((name.clone(), nid(node)));
            let _ = writeln!(body, "output {name} {}", nid(node));
        }
    }
    // a text parameter has no slider
    for (k, p) in params.iter().enumerate().filter(|(_, p)| functions::value_type(p.ty) != functions::TEXT_TYPE) {
        let (lo, hi) = (p.min.as_deref().unwrap_or("0"), p.max.as_deref().unwrap_or("1"));
        // the slider range of a vector parameter is per lane; an exposed parameter takes one number
        let first = |v: &str| v.split(',').next().unwrap_or(v).trim().to_string();
        let (lo, hi) = (first(lo), first(hi));
        let _ = writeln!(body, "expose \"{}\" {} v {lo} {hi}", p.identifier, PARAM_IDS + k);
    }

    let mut text = format!("format {}\nsize {size}\nmeta compiled \"1\"\nmeta label \"{}\"\n", GRAPH_FORMAT, desc.label.replace(['"', '#'], ""));
    // outputs whose `usertag` declares the working colour space: a host converts them back to a channel's own space
    let working: Vec<&str> = desc.outputs.iter().filter(|o| o.usertag.as_deref().is_some_and(usertag::working_space)).map(|o| o.identifier.as_str()).collect();
    if !working.is_empty() {
        let _ = writeln!(text, "meta working \"{}\"", working.join(","));
    }
    // images stored in the compiled graph travel in the text, so anything that parses it can render it
    for (file, png) in &embedded {
        let _ = writeln!(text, "resource {file} {}", b64_encode(png));
    }
    for (name, sub) in fns.subgraphs.iter().chain(programs.iter()) {
        let _ = write!(text, "subgraph {name}\n{sub}end\n");
    }
    text += &body;
    warnings.extend(fns.warnings);
    Decompiled { text: text.into(), images, image_defaults, outputs, warnings, colour_outputs }
}

/// What the pixel processors of a graph store in its value memory.
fn value_nodes(asm: &Assembly) -> Vec<ValueNode> {
    let mut out = Vec::new();
    for a in asm.atomic_nodes().into_iter().flatten() {
        let Some(expr) = a.function.as_ref().filter(|_| a.value_words > 0) else { continue };
        for ins in &expr.instrs {
            if let ("store", Operands::RefSlot(source, slot)) = (ins.name(), &ins.operands) {
                // a boolean is one word, whatever its lanes field says
                let words = if ins.value_type() == ValueType::Bool { 1 } else { u32::from(ins.components) };
                out.push(ValueNode {
                    offset: u32::from(*slot),
                    words,
                    expr: expr.clone(),
                    source: *source,
                    seed: a.seed.clone(),
                    reads_images: !a.inputs.is_empty(),
                    inputs: a.inputs.clone(),
                });
            }
        }
    }
    out
}

/// The first graph in `packages` whose name, label or url matches `graph`, decompiled.
pub fn find_and_decompile(packages: &[Vec<u8>], graph: &str, size: usize) -> Result<Option<Decompiled>, crate::decode::DecodeError> {
    use crate::decode::graph_package::GraphPackage;
    for data in packages {
        let Ok(archive) = GraphPackage::read(data) else { continue };
        for g in archive.graphs() {
            let desc = g.description()?;
            if g.name() == graph || desc.pkg_url == format!("pkg://{graph}") || desc.label == graph {
                let asm = g.assembly()?;
                return Ok(Some(decompile(&desc, &asm, size)));
            }
        }
    }
    Ok(None)
}

/// Node index bound to output `k` (the `{format, node}` pairs that follow the node table).
fn output_node(s: &crate::decode::graph_package::compiled::Section<'_>, asm: &Assembly, k: usize) -> Option<usize> {
    s.u32(asm.node_table_end + 8 * k as u32 + 4).ok().map(|v| v as usize).filter(|v| *v < asm.nodes.len())
}

fn input_value(p: &GraphInputInfo) -> String {
    let ty = functions::value_type(p.ty);
    if ty == functions::TEXT_TYPE {
        // the graph text has no escapes: a quote or a line break can't travel in a default
        let d: String = p.default.as_deref().unwrap_or("").chars().filter(|c| !matches!(c, '"' | '\n' | '\r')).collect();
        return format!("input_value name=\"{}\" type={ty} v=\"{d}\"", p.identifier);
    }
    let d: Vec<f32> = p.default.as_deref().unwrap_or("0").split(',').filter_map(|v| v.trim().parse().ok()).collect();
    let mut line = format!("input_value name=\"{}\" type={ty}", p.identifier);
    match d.as_slice() {
        [] => {}
        [v] => line += &format!(" v={v}"),
        vs => {
            for (l, v) in ["x", "y", "z", "w"].iter().zip(vs) {
                line += &format!(" {l}={v}");
            }
        }
    }
    line
}

/// One parameter onto engine parameter `key` (scalars) or `key_r`.. (four lanes, `lanes` names).
fn param(out: &mut String, key: &str, v: Option<&Value>, lanes: &[&str], desc: &Description, fns: &mut Functions, scale: f32) {
    match v {
        Some(Value::Const(w)) => {
            let f: Vec<f32> = w.iter().map(|x| f32::from_bits(*x) * scale).collect();
            if lanes.is_empty() {
                let _ = write!(out, " {key}={}", f.first().copied().unwrap_or(0.0));
            } else {
                for (l, x) in lanes.iter().zip(&f) {
                    let _ = write!(out, " {key}_{l}={x}");
                }
            }
        }
        Some(Value::Expr(e)) => {
            if lanes.is_empty() {
                let _ = write!(out, " fn.{key}=\"{}\"", fns.function(e, desc, None));
            } else {
                for (k, l) in lanes.iter().enumerate() {
                    let _ = write!(out, " fn.{key}_{l}=\"{}\"", fns.function(e, desc, Some(k)));
                }
            }
        }
        _ => {}
    }
}

/// The lanes of a parameter that scales the two axes.
const XY: [&str; 2] = ["x", "y"];

/// Whether a filter repeats the image at its edges.
fn tiling(a: &AtomicNode) -> String {
    let [x, y] = a.tiling();
    format!("tile_x={} tile_y={}", u8::from(x), u8::from(y))
}

fn input_ref(out: &mut String, slot: &str, a: &AtomicNode, k: usize) {
    if let Some(i) = a.inputs.get(k) {
        let _ = write!(out, " {slot}=@{}", image_ref(*i));
    }
}

// The compiled format's blend codes: 0 copy, 1 add, 2 subtract, 3 multiply, 4 add-sub, 5 max, 6 min, 7 switch,
// 8 divide, 9 overlay, 10 screen, 11 soft light.
// ---- the compiled format's codes, to the graph language's names (they stay here, nowhere else) ----

/// A blend node's mode: the compiled format's code to the `blend` node's `mode` name.
pub fn blend_mode(m: u32) -> &'static str {
    match m {
        1 => "add_compiled",
        2 => "subtract_compiled",
        3 => "multiply",
        4 => "add_sub_compiled",
        5 => "max",
        6 => "min",
        7 => "switch",
        8 => "divide_compiled",
        9 => "overlay",
        10 => "screen",
        11 => "soft_light_compiled",
        _ => "copy",
    }
}

/// What a node holds per channel, as the graph language's Output Format name (`None`: the graph's default, single floats).
pub fn output_format(depth: SampleDepth) -> Option<&'static str> {
    match depth {
        SampleDepth::Int8 => Some("u8_compiled"),
        SampleDepth::Int16 => Some("u16_compiled"),
        SampleDepth::Float16 => Some("f16_compiled"),
        SampleDepth::Float32 => None,
    }
}

/// The four channel lanes of a colour parameter.
const RGBA: [&str; 4] = ["r", "g", "b", "a"];

/// The lanes a per-channel parameter of `a` takes: four for a colour node, one (no suffix) for a grey one.
fn channel_lanes(a: &AtomicNode) -> &'static [&'static str] {
    if a.color { &RGBA } else { &[] }
}

/// What translating one atomic node reads and adds to: the package's description and compiled graph, its image inputs,
/// and the functions, pattern programs, embedded images and warnings collected over the whole graph.
struct NodeCx<'a> {
    desc: &'a Description,
    asm: &'a Assembly,
    images: &'a [String],
    fns: &'a mut Functions,
    programs: &'a mut Vec<(String, String)>,
    embedded: &'a mut Vec<(String, Vec<u8>)>,
    warnings: &'a mut Vec<String>,
}

/// One atomic node as a line of the graph language: its operation with its parameters ([`operation`]), then what any
/// node can carry, its sample format and its size ([`output_options`]).
#[allow(clippy::too_many_arguments, reason = "the caller's state, gathered into a `NodeCx` right away")]
fn node_line(
    a: &AtomicNode,
    desc: &Description,
    asm: &Assembly,
    images: &[String],
    fns: &mut Functions,
    programs: &mut Vec<(String, String)>,
    embedded: &mut Vec<(String, Vec<u8>)>,
    warnings: &mut Vec<String>,
) -> String {
    let mut cx = NodeCx { desc, asm, images, fns, programs, embedded, warnings };
    start_node(a, cx.fns);
    let mut l = operation(a, &mut cx);
    output_options(&mut l, a, desc, cx.fns);
    l
}

/// Sets up the functions for the node's expressions: how `rand` draws, and what runs before each of them.
fn start_node(a: &AtomicNode, fns: &mut Functions) {
    fns.rand = functions::RandContext { mode: functions::rand_mode::PARAMETER, seed: a.seed.clone() };
    // the size expression can set variables the node's other expressions read; a fx-map handles that in its program
    fns.init = if a.kind == Kind::FxMap { Vec::new() } else { a.size.iter().filter(|e| e.instrs.iter().any(|i| i.name() == "set")).cloned().collect() };
}

/// The node's operation and parameters, by its kind.
fn operation(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    match a.kind {
        Kind::Input => input_node(a, cx),
        Kind::Levels => levels(a, cx),
        Kind::Hsl => hsl(a, cx),
        Kind::Blend => blend(a, cx),
        Kind::Blur => box_blur(a, cx),
        Kind::DirectionalWarp => directional_warp(a, cx),
        Kind::GradientWarp => gradient_warp(a, cx),
        Kind::DirectionalBlur => directional_blur(a, cx),
        Kind::Sharpen => sharpen(a, cx),
        Kind::Uniform => uniform(a, cx),
        Kind::GrayscaleConversion => grayscale_conversion(a, cx),
        Kind::ChannelShuffle => channel_shuffle(a),
        Kind::Transform => transform(a, cx),
        Kind::GradientMap => gradient_map(a, cx),
        Kind::Distance => distance(a, cx),
        Kind::Normal => normal(a, cx),
        Kind::Emboss => emboss(a, cx),
        Kind::GradientDynamic => gradient_dynamic(a, cx),
        Kind::Vector => triangle_strips(a),
        Kind::Curve => curve(a),
        Kind::FxMap => fx_map(a, cx),
        Kind::Text => text(a, cx),
        // a value node has no image; expressions that read it compute it themselves
        Kind::PixelProcessor if a.value_words > 0 => "color r=0 g=0 b=0".into(),
        Kind::PixelProcessor => pixel_processor(a, cx),
        Kind::Unknown => {
            cx.warnings.push(format!("node {}: operation {:#04x} not translated", a.index, a.opcode));
            "color r=0 g=0 b=0".into()
        }
    }
}

/// An image source: a declared image input, an image stored in the graph, or black when neither can be read.
fn input_node(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, asm) = (cx.desc, cx.asm);
    let mut l = String::new();
    let word = a.image_source();
    let gray = if a.color { "" } else { " gray=1" };
    if let Some(idx) = desc.input(word).and_then(|i| cx.images.iter().position(|n| *n == i.identifier)) {
        let working = if desc.input(word).and_then(|i| i.usertag.as_deref()).is_some_and(usertag::working_space) { " working=1" } else { "" };
        l += &format!("input idx={idx}{gray}{working}");
    } else {
        // not a declared input, so the word is a section offset: an image stored in the graph
        let alpha = match a.param("alpha") {
            Some(Value::Const(w)) => w.first().and_then(|w| asm.embedded_jpeg(*w)),
            _ => None,
        };
        match asm.embedded_jpeg(word).map(|jpeg| bitmaps::add(cx.embedded, word, jpeg, alpha)) {
            Some(Ok(file)) => l += &format!("bitmap file=\"{file}\"{gray}"),
            Some(Err(e)) => {
                cx.warnings.push(format!("node {}: embedded image: {e}", a.index));
                l += "color r=0 g=0 b=0";
            }
            None => {
                let raw = asm.stored_image(a).map(|img| bitmaps::add_raw(cx.embedded, word, img.width as usize, img.height as usize, img.layout, img.samples));
                match raw {
                    Some(file) => l += &format!("bitmap file=\"{file}\"{gray}"),
                    None => {
                        cx.warnings.push(format!("node {}: image source {word:#x} not understood", a.index));
                        l += "color r=0 g=0 b=0";
                    }
                }
            }
        }
    }
    l
}

/// Levels, with the mid tone as the graph language's `levels_mid` takes it.
fn levels(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let lanes = channel_lanes(a);
    let mut l = String::from("levels_mid");
    input_ref(&mut l, "in", a, 0);
    for (src, dst) in [("in_low", "low"), ("in_high", "high"), ("in_mid", "mid"), ("out_low", "out_low"), ("out_high", "out_high")] {
        param(&mut l, dst, a.param(src), lanes, desc, fns, 1.0);
    }
    l
}

/// Hue, saturation and luminosity, each centred on 0.5.
fn hsl(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = String::from("hsl_centered");
    input_ref(&mut l, "in", a, 0);
    param(&mut l, "hue", a.param("hue"), &[], desc, fns, 1.0);
    param(&mut l, "sat", a.param("saturation"), &[], desc, fns, 1.0);
    param(&mut l, "lum", a.param("luminosity"), &[], desc, fns, 1.0);
    l
}

/// A blend of a foreground over a background: mode, how alpha blends, mask, opacity and the cropping area.
fn blend(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = String::new();
    // inputs are stored background first
    // mode word bit 8: the foreground also goes into the alpha; the foreground's alpha scales the opacity either way.
    // A grayscale image has no alpha: what its fourth channel holds here (levels invert it too) must not count.
    let alpha = a.blend_alpha().unwrap_or(BlendAlpha { foreground_alpha: false, premultiplied: false });
    let alpha_mode = if !a.color {
        1
    } else if alpha.premultiplied {
        3
    } else if alpha.foreground_alpha {
        2
    } else {
        4
    };
    l += &format!("blend mode=\"{}\" alpha_mode={alpha_mode}", blend_mode(a.blend_mode().unwrap_or(0)));
    input_ref(&mut l, "bg", a, 0);
    input_ref(&mut l, "fg", a, 1);
    if let Some(Value::Image(m)) = a.param("mask") {
        l += &format!(" mask=@{}", image_ref(*m));
    }
    param(&mut l, "opacity", a.param("opacity"), &[], desc, fns, 1.0);
    // the area that blends, the rest stays background: four 1.15 fixed-point values, (x0, y0), (x1, y1)
    match a.param("cropping") {
        Some(Value::Const(w)) => {
            let fixed = |v: u32| (v & 0xffff) as f32 / CROP_ONE;
            if let [a0, a1] = w.as_slice() {
                l += &format!(" crop_x0={} crop_y0={} crop_x1={} crop_y1={}", fixed(*a0), fixed(*a0 >> 16), fixed(*a1), fixed(*a1 >> 16));
            }
        }
        Some(Value::Expr(e)) => {
            for (k, n) in ["crop_x0", "crop_y0", "crop_x1", "crop_y1"].iter().enumerate() {
                l += &format!(" fn.{n}=\"{}\"", fns.function(e, desc, Some(k)));
            }
        }
        _ => {}
    }
    l
}

/// A box blur.
fn box_blur(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = format!("box_blur {}", tiling(a));
    input_ref(&mut l, "in", a, 0);
    param(&mut l, "aspect", a.param("aspect"), &XY, desc, fns, 1.0);
    param(&mut l, "intensity", a.param("intensity"), &[], desc, fns, 1.0);
    l
}

/// A warp along one direction, by how bright a map is.
fn directional_warp(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = format!("dir_warp {}", tiling(a));
    input_ref(&mut l, "in", a, 0);
    input_ref(&mut l, "map", a, 1);
    // of the pair the first is the brightness that moves nothing; the second had no effect
    match a.param("map_range") {
        Some(Value::Const(w)) => l += &format!(" map_offset={}", w.first().map_or(0.0, |x| f32::from_bits(*x))),
        Some(Value::Expr(e)) => l += &format!(" fn.map_offset=\"{}\"", fns.function(e, desc, Some(0))),
        _ => {}
    }
    // the pixel aspect scales the move along each axis (often `2 * 2^min(size.yx - size, 0)`)
    param(&mut l, "aspect", a.param("aspect"), &XY, desc, fns, 1.0);
    param(&mut l, "intensity", a.param("intensity"), &[], desc, fns, 1.0);
    param(&mut l, "angle", a.param("angle"), &[], desc, fns, 1.0);
    l
}

/// A warp along a map's slope.
fn gradient_warp(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = format!("grad_warp {}", tiling(a));
    input_ref(&mut l, "in", a, 0);
    input_ref(&mut l, "map", a, 1);
    param(&mut l, "aspect", a.param("aspect"), &XY, desc, fns, 1.0);
    param(&mut l, "intensity", a.param("intensity"), &[], desc, fns, 1.0);
    l
}

/// A blur along one direction.
fn directional_blur(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = format!("line_blur {}", tiling(a));
    input_ref(&mut l, "in", a, 0);
    param(&mut l, "aspect", a.param("aspect"), &XY, desc, fns, 1.0);
    param(&mut l, "intensity", a.param("intensity"), &[], desc, fns, 1.0);
    param(&mut l, "angle", a.param("angle"), &[], desc, fns, 1.0);
    l
}

/// A sharpen.
fn sharpen(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = format!("laplacian_sharpen {}", tiling(a));
    input_ref(&mut l, "in", a, 0);
    param(&mut l, "aspect", a.param("aspect"), &XY, desc, fns, 1.0);
    param(&mut l, "intensity", a.param("intensity"), &[], desc, fns, 1.0);
    l
}

/// One colour (or grey) everywhere.
fn uniform(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = String::from("color");
    match a.param("value").or(a.param("color")) {
        Some(Value::Const(w)) => {
            let f: Vec<f32> = w.iter().map(|x| f32::from_bits(*x)).collect();
            if a.color {
                for (k, x) in RGBA.iter().zip(&f) {
                    l += &format!(" {k}={x}");
                }
            } else {
                let v = f.first().copied().unwrap_or(0.0);
                l += &format!(" r={v} g={v} b={v}");
            }
        }
        Some(Value::Expr(e)) => {
            for (k, c) in ["r", "g", "b"].iter().enumerate() {
                l += &format!(" fn.{c}=\"{}\"", fns.function(e, desc, a.color.then_some(k)));
            }
        }
        _ => l += " r=0 g=0 b=0",
    }
    l
}

/// Colour to grey by a weight per channel.
fn grayscale_conversion(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = String::from("grayscale_conversion limit=1");
    input_ref(&mut l, "in", a, 0);
    // a weight above 1 counts as 1
    match a.param("weights") {
        Some(Value::Const(w)) => {
            for (k, x) in RGBA.iter().zip(w) {
                l += &format!(" {k}={}", f32::from_bits(*x).min(1.0));
            }
        }
        Some(Value::Expr(e)) => {
            for (k, c) in RGBA.iter().enumerate() {
                l += &format!(" fn.{c}=\"{}\"", fns.function(e, desc, Some(k)));
            }
        }
        // measured: a third of each, exactly
        _ => l += " r=0.33333334 g=0.33333334 b=0.33333334",
    }
    l
}

/// Each output channel taken from a channel of one of two inputs.
fn channel_shuffle(a: &AtomicNode) -> String {
    let mut l = String::from("channel_shuffle");
    input_ref(&mut l, "in", a, 0);
    input_ref(&mut l, "in2", a, 1);
    for (c, src) in RGBA.iter().zip(a.channel_sources().unwrap_or([0, 1, 2, 3])) {
        l += &format!(" src_{c}={src}");
    }
    l
}

/// A 2 x 2 matrix and an offset, with how the image is sampled and what shows outside it.
fn transform(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let s = a.transform_sampling().unwrap_or(TransformSampling { mip_level: TRANSFORM_DEFAULT_MIP, bilinear: true });
    let mut l = format!("transform_matrix {} filter={} mip={}", tiling(a), u8::from(s.bilinear), s.mip_level);
    input_ref(&mut l, "in", a, 0);
    // what shows outside the image on an axis that doesn't repeat, black with no alpha by default
    if let Some(Value::Const(w)) = a.param("background") {
        let c: Vec<f32> = w.iter().map(|x| f32::from_bits(*x)).collect();
        let c = if let [v] = c[..] { [v, v, v, 1.0] } else { [0, 1, 2, 3].map(|k| c.get(k).copied().unwrap_or(0.0)) };
        l += &format!(" bg_r={} bg_g={} bg_b={} bg_a={}", c[0], c[1], c[2], c[3]);
    }
    match a.param("matrix") {
        Some(Value::Const(w)) => {
            for (k, x) in ["m00", "m01", "m10", "m11"].iter().zip(w) {
                l += &format!(" {k}={}", f32::from_bits(*x));
            }
        }
        Some(Value::Expr(e)) => {
            for (k, n) in ["m00", "m01", "m10", "m11"].iter().enumerate() {
                l += &format!(" fn.{n}=\"{}\"", fns.function(e, desc, Some(k)));
            }
        }
        _ => {}
    }
    match a.param("offset") {
        Some(Value::Const(w)) => {
            for (k, x) in ["offset_x", "offset_y"].iter().zip(w) {
                l += &format!(" {k}={}", f32::from_bits(*x));
            }
        }
        Some(Value::Expr(e)) => {
            for (k, n) in ["offset_x", "offset_y"].iter().enumerate() {
                l += &format!(" fn.{n}=\"{}\"", fns.function(e, desc, Some(k)));
            }
        }
        _ => {}
    }
    l
}

/// The sfnt version of a font with OpenType (CFF) outlines.
const OPENTYPE_CFF: &[u8] = b"OTTO";

/// Text drawn with the font stored in the graph: the text from the graph input it names, position (an offset from the
/// middle, y up), size and alignment as the graph runtime's `text` node takes them.
fn text(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, asm) = (cx.desc, cx.asm);
    // the position's y counts up the image (measured on Day 7: the label's text sits 0.1 of the tip lower otherwise)
    let mut l = String::from("text y_up=1");
    let font = match a.param("font") {
        Some(Value::Const(w)) => w.first().copied(),
        _ => None,
    };
    match font.and_then(|off| asm.embedded_font(off).map(|ttf| (off, ttf))) {
        // the graph runtime reads TrueType outlines only: an OpenType (CFF) face draws with the built-in one instead of
        // failing the render (font_segment14)
        Some((_, ttf)) if ttf.starts_with(OPENTYPE_CFF) => {
            cx.warnings.push(format!("node {}: the text's font has OpenType (CFF) outlines (the built-in face draws it)", a.index));
        }
        Some((off, ttf)) => {
            let file = format!("font_{off:x}.ttf");
            if !cx.embedded.iter().any(|(name, _)| *name == file) {
                cx.embedded.push((file.clone(), ttf.to_vec()));
            }
            l += &format!(" font=\"{file}\"");
        }
        None => cx.warnings.push(format!("node {}: the text's font is not understood (the built-in face draws it)", a.index)),
    }
    match a.param("text_input") {
        Some(Value::Const(w)) if w.first().is_some_and(|uid| desc.input(*uid).is_some()) => {
            let uid = w.first().copied().unwrap_or(0);
            l += &format!(" fn.text=\"{}\"", cx.fns.input_function(uid, desc));
        }
        _ => cx.warnings.push(format!("node {}: the text's source is not understood (it draws no text)", a.index)),
    }
    match a.param("position") {
        Some(Value::Const(w)) => {
            for (k, x) in XY.iter().zip(w) {
                l += &format!(" {k}={}", f32::from_bits(*x));
            }
        }
        Some(Value::Expr(e)) => {
            for (lane, k) in XY.iter().enumerate() {
                l += &format!(" fn.{k}=\"{}\"", cx.fns.function(e, desc, Some(lane)));
            }
        }
        _ => {}
    }
    param(&mut l, "size", a.param("fontsize"), &[], desc, cx.fns, 1.0);
    match a.param("alignment") {
        Some(Value::Const(w)) => l += &format!(" align={}", w.first().copied().unwrap_or(1)),
        v => param(&mut l, "align", v, &[], desc, cx.fns, 1.0),
    }
    l
}

/// Grey to colour through a table of keys.
fn gradient_map(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let asm = cx.asm;
    // flags bit 8 says the keys carry a midpoint. Up to format 6 the midpoint does nothing and the lines are straight,
    // without one the steps are smooth. From format 8 lines without one are straight, and a midpoint bends them in
    // a way not understood yet.
    let midpoints = a.key_midpoints();
    if asm.version >= 8 && midpoints {
        cx.warnings.push(format!("node {}: gradient map midpoints of format {} read as straight lines", a.index, asm.version));
    }
    let smooth = asm.version < 8 && !midpoints;
    let mut l = String::from(if smooth { "colorize interp=1 use_alpha=1 table=1" } else { "colorize interp=0 use_alpha=1 table=1" });
    input_ref(&mut l, "in", a, 0);
    let mut keys: Vec<_> = a.keys.iter().collect();
    keys.sort_by(|x, y| x.position.total_cmp(&y.position));
    // of keys at one position only the first counts
    keys.dedup_by(|later, first| later.position == first.position);
    for (k, key) in keys.iter().enumerate() {
        l += &format!(" p{k}={} r{k}={} g{k}={} b{k}={} a{k}={}", key.position, key.color[0], key.color[1], key.color[2], key.color[3]);
    }
    l
}

/// The distance to the nearest lit pixel.
fn distance(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = format!("distance_map fade={} {}", u8::from(a.distance_fades().unwrap_or(false)), tiling(a));
    input_ref(&mut l, "in", a, 0);
    input_ref(&mut l, "source", a, 1);
    param(&mut l, "distance", a.param("distance"), &[], desc, fns, 1.0);
    param(&mut l, "aspect", a.param("aspect"), &["x", "y"], desc, fns, 1.0);
    l
}

/// A normal map from a height map.
fn normal(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let o = a.normal_options().unwrap_or(NormalOptions { height_in_alpha: false, flip_y: false });
    let mut l = format!("normal_map alpha={} flip_y={}", u8::from(o.height_in_alpha), u8::from(o.flip_y));
    input_ref(&mut l, "in", a, 0);
    param(&mut l, "intensity", a.param("intensity"), &[], desc, fns, 1.0);
    param(&mut l, "flip_y", a.param("flip_y"), &[], desc, fns, 1.0);
    param(&mut l, "aspect", a.param("aspect"), &["x", "y"], desc, fns, 1.0);
    l
}

/// An emboss: an image lit from an angle by a height map.
fn emboss(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = String::from("emboss_map");
    input_ref(&mut l, "in", a, 0);
    input_ref(&mut l, "map", a, 1);
    param(&mut l, "intensity", a.param("intensity"), &[], desc, fns, 1.0);
    param(&mut l, "angle", a.param("angle"), &[], desc, fns, 1.0);
    param(&mut l, "aspect", a.param("aspect"), &["x", "y"], desc, fns, 1.0);
    l
}

/// Grey to colour through a row (or column) of a gradient image.
fn gradient_dynamic(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = format!("gradient_dynamic vertical={}", u8::from(a.gradient_vertical()));
    input_ref(&mut l, "in", a, 0);
    input_ref(&mut l, "grad", a, 1);
    param(&mut l, "position", a.param("position"), &[], desc, fns, 1.0);
    l
}

/// Vector shapes: the node's triangle strips, as hex words.
fn triangle_strips(a: &AtomicNode) -> String {
    let mut hex = String::new();
    for strip in &a.strips {
        for v in strip {
            let _ = write!(hex, "{v:08x}");
        }
        hex.push(' ');
    }
    format!("triangle_strips strips=\"{}\"", hex.trim_end())
}

/// A curve through keys with tangent handles.
fn curve(a: &AtomicNode) -> String {
    let mut l = format!("curve_keys ramp={} n={}", u8::from(a.curve_ramp()), a.curve.len());
    input_ref(&mut l, "in", a, 0);
    for (k, c) in a.curve.iter().enumerate() {
        l += &format!(" x{k}={} y{k}={} lx{k}={} ly{k}={} rx{k}={} ry{k}={}", c.point[0], c.point[1], c.left[0], c.left[1], c.right[0], c.right[1]);
    }
    l
}

/// A fx-map: its program as a pattern program subgraph, with its constant parameters and image inputs on the node.
fn fx_map(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = String::new();
    let name = format!("pattern{}", cx.programs.len() + 1);
    match &a.program {
        Some(p) => {
            // expressions run inside the program, in this order: they can set variables later ones read
            let mut init: Vec<(&str, &crate::decode::graph_package::compiled::Expr)> = a.seed.iter().map(|e| ("seed", e)).collect();
            init.extend(a.size.iter().filter(|e| e.as_input().is_none()).map(|e| ("init", e)));
            for (key, name) in [("background", "p0"), ("roughness", "p1"), ("opacity", "p2"), ("frame", "frame_matrix"), ("frame_offset", "frame_offset")] {
                if let Some(Value::Expr(e)) = a.param(name) {
                    init.push((key, e));
                }
            }
            cx.programs.push((name.clone(), programs::program(p, &init, desc, fns)));
            l += &format!("pattern_program program=\"{name}\"");
            if a.color {
                l += " color=1";
            }
            for (key, name, lanes) in [
                ("background", "p0", &[][..]),
                ("roughness", "p1", &[][..]),
                ("opacity", "p2", &[][..]),
                ("frame", "frame_matrix", &["x0", "x1", "y0", "y1"][..]),
                ("repeat", "frame_offset", &["x0", "x1", "y0", "y1"][..]),
            ] {
                if let Some(v @ Value::Const(_)) = a.param(name) {
                    param(&mut l, key, Some(v), lanes, desc, fns, 1.0);
                }
            }
            if let (true, Some(Value::Const(w))) = (a.color, a.param("p0")) {
                // what the gray form above took from the first word
                for (k, x) in RGBA.iter().zip(w) {
                    let _ = write!(l, " background_{k}={}", f32::from_bits(*x));
                }
            }
            if let Some(Value::Image(i)) = a.param("p0") {
                l += &format!(" bg=@{}", image_ref(*i));
            }
            l += &format!(" {}", tiling(a));
            for k in 0..a.inputs.len().min(32) {
                input_ref(&mut l, &functions::slot(k as u16), a, k);
            }
            if a.inputs.len() > 32 {
                cx.warnings.push(format!("node {}: {} image inputs, only the first 32 translated", a.index, a.inputs.len()));
            }
        }
        None => l += "color r=0 g=0 b=0",
    }
    l
}

/// A pixel processor that makes an image: its function, with the image slots it samples (its own inputs, then those
/// of the value nodes it computes in place).
fn pixel_processor(a: &AtomicNode, cx: &mut NodeCx<'_>) -> String {
    let (desc, fns) = (cx.desc, &mut *cx.fns);
    let mut l = String::new();
    match &a.function {
        Some(f) => {
            fns.rand.mode = functions::rand_mode::PIXEL;
            let own = a.inputs.len().min(32);
            let (name, extra) = functions::with_host(own, || fns.function(f, desc, None));
            l += &format!("pixel_processor fn=\"{name}\"");
            fns.rand.mode = functions::rand_mode::PARAMETER;
            for (k, i) in a.inputs.iter().chain(&extra).enumerate().take(32) {
                l += &format!(" {}=@{}", functions::slot(k as u16), image_ref(*i));
            }
            if own + extra.len() > 32 {
                cx.warnings.push(format!("node {}: a value read from an image needs slot {} of 32", a.index, own + extra.len()));
            }
        }
        None => l += "color r=0 g=0 b=0",
    }
    l
}

/// What any node can carry after its operation: the sample format it holds and its size, fixed, the graph's, or computed.
fn output_options(l: &mut String, a: &AtomicNode, desc: &Description, fns: &mut Functions) {
    // what the node holds per channel: 8 bits, 16 bits, half float or single (the graph's default)
    if let Some(format) = output_format(a.sample_depth()) {
        let _ = write!(l, " format=\"{format}\"");
    }
    // a stored image too: it keeps its own size whatever the graph's, and what reads it reduces it
    if a.size.is_none() && a.kind != Kind::Unknown {
        let [x, y] = a.size_log2();
        let _ = write!(l, " osize_log2_x={x} osize_log2_y={y}");
    }
    if let Some(size) = &a.size {
        match size.as_input() {
            // the graph's size, whatever size the inputs have
            Some(uid) if desc.input(uid).is_some_and(|i| i.identifier == "$outputsize") => *l += " osize_mode=1",
            Some(_) => {}
            None => {
                fns.init.clear();
                for (k, axis) in ["x", "y"].iter().enumerate() {
                    *l += &format!(" fn.osize_log2_{axis}=\"{}\"", fns.function(size, desc, Some(k)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

/// Standard base64 with padding, as the graph text carries its `resource` files.
fn b64_encode(d: &[u8]) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(d.len() * 4 / 3 + 4);
    // (a 6-bit index is always in the table)
    let digit = |v: u32| B64.get(v as usize & 63).copied().unwrap_or(b'=') as char;
    for c in d.chunks(3) {
        let byte = |k: usize| u32::from(c.get(k).copied().unwrap_or(0));
        let n = byte(0) << 16 | byte(1) << 8 | byte(2);
        s.push(digit(n >> 18));
        s.push(digit(n >> 12));
        s.push(if c.len() > 1 { digit(n >> 6) } else { '=' });
        s.push(if c.len() > 2 { digit(n) } else { '=' });
    }
    s
}
