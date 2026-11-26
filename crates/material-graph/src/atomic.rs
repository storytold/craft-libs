// SPDX-License-Identifier: MIT OR Apache-2.0
//! Image operations with the parameter conventions of compiled node graphs (what the importer decompiles from
//! a project's procedurals): levels with a midpoint and per-channel values, HSL shifts centred on 0.5 and
//! transforms given as a 2×2 matrix and an offset. `compute` returns Ok(None) for kinds it does not know.

use crate::graph::{Error, Node, Spec};
use crate::math;
use crate::measured::{COMPILED_8_BIT, COMPILED_16_BIT, COMPILED_HALF, FINE_16, FINE_16_TIE_DOWN, INTENSITY_SPAN, STEPS_8, STEPS_16, WEIGHT_STEPS};
use crate::tex::{Image, Pixel, Tex, get};
use std::collections::BTreeMap;
use std::sync::Arc as Rc;

/// An intensity a node's text leaves out. Blur: 10, measurement "One-effect projects" › "Node rules" ("the intensity silently
/// becomes its default, 10"). Directional warp: 10, measurement "Fx-maps with random numbers, five shared causes" (dir_warp
/// "default 10"). Gradient warp: 1, measurement "Operations identified with it" (0x0e/0x0f "Default intensity 1").
const BLUR_DEFAULT_INTENSITY: f32 = 10.0;
const DIR_WARP_DEFAULT_INTENSITY: f32 = 10.0;
const GRAD_WARP_DEFAULT_INTENSITY: f32 = 1.0;
/// Directional blur and sharpen: no measurement gives these; guesses (the blur's, and the identity scale).
const LINE_BLUR_DEFAULT_INTENSITY: f32 = 10.0;
const SHARPEN_DEFAULT_INTENSITY: f32 = 1.0;

/// A transform's mip code that picks the level from the scale; 30 does the same. Measurement "Fx-map drawing" ("Transforms
/// (V)": bits 0-4 mip level, 31 automatic) and "Fx-map frames, draw order and small rules" › "Images" ("Transform mip
/// code 30 acts as 31").
const MIP_AUTOMATIC: u32 = 31;
const MIP_AUTOMATIC_TOO: u32 = 30;
/// Doubling: each new pixel this much of the nearer pixel and the rest of the next. Measurement "Transforms" ("each new pixel
/// 3/4 nearer + 1/4 next").
const DOUBLING_NEAR: f32 = 0.75;
const DOUBLING_FAR: f32 = 0.25;
/// A compiled box along a row: up to this width (centre and two reads a side) without the pyramid. Measurement "What nodes hold
/// and how they read" ("Up to 5 pixels: centre + one read each side").
const SHORT_BOX_WIDTH: i64 = 5;
/// A row value this close to a whole 16-bit step is that step: single precision brings a step back up to 0.004 off it (a
/// guard, not measured).
const WHOLE_STEP_SLACK: f64 = 0.01;
/// A line blur's line is half a pixel shorter at each end than the intensity, and its box reaches half a pixel past the
/// line. Measurement "Directional blur, exact form" (`max(rx-.5,0)`, `R = .5 + max`).
const LINE_HALF_PIXEL: f32 = 0.5;
/// A sine or cosine this small counts as an exact axis (a guard, not measured).
const AXIS_SLACK: f32 = 1e-4;
/// A line at 45 degrees is a tie: the relative slack that sends it to y. Measurement "Directional blur, exact form" ("tie to
/// y"); the size of the slack is a guard.
const TIE_SLACK: f64 = 1e-6;
/// Levels: the midpoint sets a power of `1 + 16 |mid - 0.5|`. Measurement "Compiled opcodes onto native atomics" (levels:
/// "power 1+16|mid-.5| measured").
const LEVELS_MID_POWER: f32 = 16.0;
/// Levels with low and high this close is a step (a guard; the step itself: measurement "Fx-map frames, draw order and small
/// rules" › "Parameters and levels").
const LEVELS_STEP_SLACK: f32 = 1e-6;
/// HSL: twice a parameter's distance from 0.5 is added. Not measured; from the compiled format's node, a guess.
const HSL_SHIFT_GAIN: f32 = 2.0;

/// Inputs of `pattern_program`: the images its program draws, and the image it draws on.
const PROGRAM_SLOTS: &[&str] = &[
    "in", "in2", "in3", "in4", "in5", "in6", "in7", "in8", "in9", "in10", "in11", "in12", "in13", "in14", "in15", "in16", "in17", "in18", "in19",
    "in20", "in21", "in22", "in23", "in24", "in25", "in26", "in27", "in28", "in29", "in30", "in31", "in32", "bg",
];

pub const SPECS: &[Spec] = &[
    Spec {
        kind: "levels_mid",
        req: &["in"],
        opt: &[],
        doc: "levels with a midpoint: low high mid out_low out_high; per channel with suffix _r _g _b _a (e.g. low_r)",
    },
    Spec {
        kind: "hsl_centered",
        req: &["in"],
        opt: &[],
        doc: "hue, saturation and lightness shifts, 0.5 = unchanged, twice the distance from 0.5 is added: hue (turns) sat lum",
    },
    Spec {
        kind: "transform_matrix",
        req: &["in"],
        opt: &[],
        doc: "transform by a matrix around the centre, reading the input at (m00 x + m10 y + offset_x, m01 x + m11 y + offset_y) from the centre: tile_x tile_y (1 repeat, 0 bg_r bg_g bg_b bg_a outside, default none) filter (1 bilinear, 0 nearest) mip (level, 31 from the scale)",
    },
    Spec {
        kind: "dir_warp",
        req: &["in", "map"],
        opt: &[],
        doc: "move each pixel along a direction by the brightness of map: intensity (1/256 of the image per unit of map), angle (turns), map_offset (brightness that moves nothing), tile_x tile_y (1 repeat, 0 repeat the edge)",
    },
    Spec {
        kind: "grad_warp",
        req: &["in", "map"],
        opt: &[],
        doc: "move each pixel along the slope of map: intensity (default 1; 1/256 of the image per unit of slope in uv), aspect_x, aspect_y (default 1), tile_x tile_y (1 repeat, 0 keep the edge: no slope across it, edge value read)",
    },
    Spec {
        kind: "box_blur",
        req: &["in"],
        opt: &[],
        doc: "blur with a box in both axes reaching intensity pixels of a 256 pixel image each way: intensity aspect_x aspect_y (scale it per axis) tile_x tile_y (1 repeat, 0 repeat the edge)",
    },
    Spec {
        kind: "line_blur",
        req: &["in"],
        opt: &[],
        doc: "average along a line reaching intensity pixels of a 256 pixel image each way, by shearing the image, a box along the longer axis and shearing back: intensity aspect_x aspect_y (scale it per axis) angle (turns) tile_x tile_y (1 repeat, 0 repeat the edge)",
    },
    Spec {
        kind: "laplacian_sharpen",
        req: &["in"],
        opt: &[],
        doc: "add intensity times the difference to the four neighbours, limited to 0..1: intensity aspect_x aspect_y (scale it per axis) tile_x tile_y (1 repeat, 0 repeat the edge)",
    },
    Spec {
        kind: "pattern_program",
        req: &[],
        opt: PROGRAM_SLOTS,
        doc: "run a pattern program: program=\"name\" seed color(0/1); bg is an image the canvas starts from",
    },
];

/// Run a node of this module. Ok(None) = not one of ours.
pub fn compute(kind: &str, id: u32, n: &Node, ins: &BTreeMap<String, Rc<Tex>>, ctx: &crate::patterns::Context) -> Result<Option<Tex>, Error> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let need = |slot: &str| -> Result<&Tex, Error> { ins.get(slot).map(|r| r.as_ref()).ok_or_else(|| Error::MissingInput(id, slot.to_string())) };
    Ok(Some(match kind {
        "levels_mid" => {
            let (params, float) = levels_params(n);
            need("in")?.map(move |p| {
                let mut o = p;
                for (k, v) in o.iter_mut().enumerate() {
                    if let Some(q) = params.get(k) {
                        *v = if float { levels_value_unbounded(*v, q) } else { levels_value(*v, q) };
                    }
                }
                o
            })
        }
        "hsl_centered" => {
            let (h, s, l) = (f("hue", 0.5), f("sat", 0.5), f("lum", 0.5));
            need("in")?.map(move |p| hsl_shift(p, h, s, l))
        }
        "transform_matrix" => {
            let bg = [f("bg_r", 0.0), f("bg_g", 0.0), f("bg_b", 0.0), f("bg_a", 0.0)];
            transform_matrix(n, need("in")?, ctx.size, bg, crate::tex::ToImage)
        }
        "box_blur" => box_blur(n, need("in")?),
        "line_blur" => line_blur_node(id, n, need("in")?)?,
        "laplacian_sharpen" => {
            let (i, tx, ty) = (f("intensity", SHARPEN_DEFAULT_INTENSITY), f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5);
            let t = need("in")?;
            let weights = [i * f("aspect_x", 1.0), i * f("aspect_y", 1.0)];
            let [w, h] = ctx.size;
            if (t.w, t.h) == (w, h) {
                laplacian_sharpen(t, weights, tx, ty)
            } else {
                // an input of another size is read at each result pixel, its neighbours one input pixel away
                let (dx, dy) = (1.0 / t.w as f32, 1.0 / t.h as f32);
                Tex::from_fn(w, h, |u, v| {
                    let read = |a: f32, b: f32| sample_edge(t, a, b, tx, ty);
                    let (p, l, r, up, down) = (read(u, v), read(u - dx, v), read(u + dx, v), read(u, v - dy), read(u, v + dy));
                    let mut o = [0.0; 4];
                    for (c, o) in o.iter_mut().enumerate() {
                        let (p, l, r, up, down) = (get(&p, c), get(&l, c), get(&r, c), get(&up, c), get(&down, c));
                        *o = (p + weights[0] * (2.0 * p - l - r) + weights[1] * (2.0 * p - up - down)).clamp(0.0, 1.0);
                    }
                    o
                })
            }
        }
        "dir_warp" => dir_warp(n, need("in")?, need("map")?),
        "grad_warp" => grad_warp(n, need("in")?, need("map")?, ctx.size),
        "pattern_program" => {
            let name = n.strs.get("program").cloned().unwrap_or_default();
            let program = ctx.lib.get(&name).ok_or_else(|| Error::BadParam(id, format!("no pattern program named '{name}'")))?;
            // inputs by position: a missing one leaves a gap so later indices stay right
            let mut images: Vec<&Tex> = Vec::new();
            for (k, slot) in crate::graph::IMAGE_SLOTS.iter().enumerate() {
                if let Some(t) = ins.get(*slot) {
                    images.resize(k, t.as_ref());
                    images.push(t.as_ref());
                }
            }
            crate::patterns::render(program, ctx, f("seed", 0.0) as u32, &images, ins.get("bg").map(|t| t.as_ref()), pattern_frame(n))
                .map_err(|e| if crate::context::cancelled() { Error::Cancelled } else { Error::BadParam(id, e) })?
        }
        other => return crate::compiled_ops::compute(other, id, n, ins, ctx.size),
    }))
}

/// A `pattern_program` node's frame.
pub(crate) fn pattern_frame(n: &Node) -> crate::patterns::Frame {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    crate::patterns::Frame {
        background: f("background", 0.0),
        roughness: f("roughness", 1.0),
        opacity: f("opacity", 1.0),
        region: [f("frame_x0", 0.0), f("frame_x1", 1.0), f("frame_y0", 0.0), f("frame_y1", 1.0)],
        tile: [f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5],
        repeat: [f("repeat_x0", 0.0), f("repeat_x1", 1.0), f("repeat_y0", 0.0), f("repeat_y1", 1.0)],
        color: f("color", 0.0) > 0.5,
        background_color: n
            .nums
            .contains_key("background_r")
            .then(|| [f("background_r", 0.0), f("background_g", 0.0), f("background_b", 0.0), f("background_a", 1.0)]),
    }
}

fn resident_affine(
    n: &Node,
    [w, h]: [usize; 2],
    size: [usize; 2],
    tile: [bool; 2],
    alpha: f32,
) -> Option<crate::accelerator::FilterOperation<'static>> {
    use crate::accelerator::FilterOperation;
    let value = |name, default| n.nums.get(name).copied().unwrap_or(default);
    let (matrix, offset) = transform_matrix_coordinates(n);
    let bilinear = value("filter", 1.0) > 0.5;
    let mip = value("mip", MIP_AUTOMATIC as f32).round() as u32;
    if transform_matrix_bg(n) != [0.0; 4] || matrix.into_iter().chain(offset).any(|v| !(-16.0..=16.0).contains(&v)) {
        return None;
    }
    // Leave per-axis resampling and repeated doubling to their specialized paths.
    if bilinear && mip >= MIP_AUTOMATIC_TOO && offset == [0.0; 2] && is_axis_swap(matrix) {
        let [tw, th] = if matrix[0] == 0.0 { [size[1], size[0]] } else { size };
        if ((w > tw || h > th) && w * th != h * tw) || (mip == MIP_AUTOMATIC && (w < size[0] || h < size[1])) {
            return None;
        }
    }
    let [m00, m01, m10, m11] = matrix;
    let scaled = [m00 * w as f32 / size[0] as f32, m01 * h as f32 / size[0] as f32, m10 * w as f32 / size[1] as f32, m11 * h as f32 / size[1] as f32];
    let level = transform_mip(mip, scaled);
    let mip_side = if level == 0 {
        None
    } else {
        let side = w.checked_shr(level).unwrap_or_default();
        if w != h || side < 2 {
            return None;
        }
        Some(side)
    };
    let reduces = math::hypot(scaled[0], scaled[1]).max(math::hypot(scaled[2], scaled[3])) > 1.0;
    let edge = [mip_side.unwrap_or(w), mip_side.unwrap_or(h)];
    let edge_fade = if mip < MIP_AUTOMATIC_TOO && level > 0 && reduces { edge } else { [edge[0].max(size[0]), edge[1].max(size[1])] };
    if alpha != 0.0 && tile != [true; 2] && !affine_covers_image(matrix, offset, size, tile, bilinear.then_some(edge_fade)) {
        return None;
    }
    Some(FilterOperation::Affine { matrix, offset, bilinear, mip_side, edge_fade })
}

/// The compiled grayscale filter subset that can be evaluated without leaving the GPU.
pub(crate) fn resident_filter<'a>(
    n: &Node,
    size: [usize; 2],
    div: f32,
    source: crate::accelerator::GreyInput<'a>,
    map: Option<crate::accelerator::GreyInput<'a>>,
) -> Option<crate::accelerator::ResidentFilter<'a>> {
    use crate::accelerator::{FilterOperation, ResidentFilter};
    let packed = |input: crate::accelerator::GreyInput<'_>| {
        let (w, h, div, alpha) = input.info();
        w >= 2 && w.is_power_of_two() && h.is_power_of_two() && [STEPS_8 as f32, STEPS_16 as f32].contains(&div) && crate::tex::grey_alpha_ok(alpha)
    };
    if !packed(source) || !size.into_iter().all(usize::is_power_of_two) || size[0] < 2 {
        return None;
    }
    let (w, h, _, alpha) = source.info();
    // The packed shader can run the wider curvature filters at 2K. Keep the work
    // of a direct-tap pass bounded on larger images.
    let max_radius = if w.saturating_mul(h) <= 2048 * 2048 { 48.0 } else { 12.0 };
    if !matches!(n.kind.as_str(), "grad_warp" | "transform_matrix") && size != [w, h] {
        return None;
    }
    let value = |name, default| n.nums.get(name).copied().unwrap_or(default);
    let quantized_blur = matches!(n.nums.get("ofmt").map(|format| format.round() as u32), Some(COMPILED_8_BIT | COMPILED_16_BIT));
    let tile = [value("tile_x", 1.0) > 0.5, value("tile_y", 1.0) > 0.5];
    let aspect = [value("aspect_x", 1.0), value("aspect_y", 1.0)];
    let angle = value("angle", 0.0);
    if !aspect.into_iter().chain([angle]).all(f32::is_finite) {
        return None;
    }
    let radii = |intensity: f32| [intensity * aspect[0] * w as f32 / INTENSITY_SPAN, intensity * aspect[1] * h as f32 / INTENSITY_SPAN];
    let operation = match n.kind.as_str() {
        "transform_matrix" => resident_affine(n, [w, h], size, tile, alpha)?,
        "laplacian_sharpen" => {
            let weights = aspect.map(|axis| value("intensity", SHARPEN_DEFAULT_INTENSITY) * axis);
            if !weights.into_iter().all(f32::is_finite) {
                return None;
            }
            FilterOperation::Sharpen { weights }
        }
        "dir_warp" => {
            let map = map.filter(|input| packed(*input))?;
            let crate::accelerator::DirectionalWarpParams { intensity, offset, direction, aspect, .. } = resident_directional_warp(n)?;
            FilterOperation::DirectionalWarp { map, intensity, offset, direction, aspect }
        }
        "grad_warp" => {
            let map = map.filter(|input| packed(*input))?;
            let intensity = value("intensity", GRAD_WARP_DEFAULT_INTENSITY) / INTENSITY_SPAN;
            let (mw, mh, ..) = map.info();
            let displacement = aspect.map(|axis| intensity * axis);
            if displacement.into_iter().zip([mw, mh]).any(|(axis, width)| !axis.is_finite() || axis.abs() * width as f32 > 8192.0) {
                return None;
            }
            FilterOperation::GradientWarp { map, displacement }
        }
        "box_blur" if quantized_blur => {
            let radius = radii(value("intensity", BLUR_DEFAULT_INTENSITY)).map(f32::abs);
            if radius.into_iter().any(|r| !(0.0..=max_radius).contains(&r)) {
                return None;
            }
            FilterOperation::Box { radius }
        }
        "line_blur" if quantized_blur => {
            let [rx, ry] = radii(value("intensity", LINE_BLUR_DEFAULT_INTENSITY));
            let (sin, cos) = math::sin_cos(angle * std::f32::consts::TAU);
            if ![rx, ry, sin, cos].into_iter().all(f32::is_finite) {
                return None;
            }
            if sin.abs() < AXIS_SLACK || cos.abs() < AXIS_SLACK {
                let radius = if sin.abs() < AXIS_SLACK { [rx.abs(), 0.0] } else { [0.0, ry.abs()] };
                if radius.into_iter().any(|r| r > max_radius) {
                    return None;
                }
                FilterOperation::Box { radius }
            } else {
                let (dx, dy) = ((rx - LINE_HALF_PIXEL).max(0.0) * cos, (ry - LINE_HALF_PIXEL).max(0.0) * sin);
                let longest = dx.abs().max(dy.abs());
                let along_y = f64::from(dx.abs()) * h as f64 <= f64::from(dy.abs()) * w as f64 * (1.0 + TIE_SLACK);
                if longest == 0.0 {
                    FilterOperation::Box { radius: [0.0; 2] }
                } else if longest < 1.0 {
                    let slope = if along_y { dx / dy * h as f32 / w as f32 } else { dy / dx * w as f32 / h as f32 };
                    FilterOperation::ShortLine { major: usize::from(along_y), slope, radius: LINE_HALF_PIXEL + longest }
                } else {
                    let (major, minor, slope, radius, repeat_rows) = if along_y {
                        (h, w, dx / dy, LINE_HALF_PIXEL + dy.abs(), tile[0])
                    } else {
                        (w, h, dy / dx, LINE_HALF_PIXEL + dx.abs(), tile[1])
                    };
                    if radius > max_radius || !slope.is_finite() {
                        return None;
                    }
                    let pad = radius.ceil() as usize + 2;
                    let extra =
                        if repeat_rows { 0 } else { (f64::from(slope.abs()) * (major as f64 / 2.0 + pad as f64 + f64::from(radius))) as usize + 3 };
                    if minor + 2 * extra > MAX_SHEARED_SIDE {
                        return None;
                    }
                    FilterOperation::ShearedLine { major: usize::from(along_y), slope, radius }
                }
            }
        }
        _ => return None,
    };
    Some(ResidentFilter { source, size, tile, div, operation })
}

/// A `box_blur` node on an image of pixels `P`.
pub(crate) fn box_blur<P: Pixel>(n: &Node, t: &Image<P>) -> Image<P> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let (i, tx, ty) = (f("intensity", BLUR_DEFAULT_INTENSITY), f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5);
    let (ax, ay) = (f("aspect_x", 1.0), f("aspect_y", 1.0));
    let (rx, ry) = (i * ax * t.w as f32 / INTENSITY_SPAN, i * ay * t.h as f32 / INTENSITY_SPAN);
    let ofmt = f("ofmt", 0.0).round() as u32;
    if matches!(ofmt, COMPILED_8_BIT | COMPILED_16_BIT | COMPILED_HALF) {
        // a node holding 8 bits runs across first and holds the pass in 8 bits too (measured on synthetic graphs at radius 0.9 to 2.4:
        // 99 to 100% of pixels exact, where one rounding at the end gets 86 to 92%)
        held_blur(t, rx, ry, tx, ty, ofmt)
    } else {
        box_axis(&box_columns(t, ry, ty), rx, tx)
    }
}

/// A `line_blur` node on an image of pixels `P`.
pub(crate) fn line_blur_node<P: Pixel>(id: u32, n: &Node, t: &Image<P>) -> Result<Image<P>, Error> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let (i, tx, ty) = (f("intensity", LINE_BLUR_DEFAULT_INTENSITY), f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5);
    let aspect = [f("aspect_x", 1.0), f("aspect_y", 1.0)];
    let angle = f("angle", 0.0);
    let format = f("ofmt", 0.0).round() as u32;
    let (sn, cs) = math::sin_cos(angle * std::f32::consts::TAU);
    Ok(if sn.abs() < AXIS_SLACK {
        compiled_box_axis(t, i * aspect[0] * t.w as f32 / INTENSITY_SPAN, tx, format)
    } else if cs.abs() < AXIS_SLACK {
        transposed(&compiled_box_axis(&transposed(t), i * aspect[1] * t.h as f32 / INTENSITY_SPAN, ty, format))
    } else {
        line_blur(t, i, aspect, angle, tx, ty).ok_or_else(|| Error::BadParam(id, format!("line blur longer than {MAX_SHEARED_SIDE} pixels")))?
    })
}

fn directional_warp_params(n: &Node) -> crate::accelerator::DirectionalWarpParams {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let (sn, cs) = math::sin_cos(f("angle", 0.0) * std::f32::consts::TAU);
    // the node's pixel aspect scales the move along each axis (measured by fitting the move to the reference renderer's taps:
    // a constant (2, 2) or (0.5, 1), or `2 * 2^min(size.yx - size, 0)`, is the factor; no aspect moves by 1)
    crate::accelerator::DirectionalWarpParams {
        intensity: f("intensity", DIR_WARP_DEFAULT_INTENSITY) / INTENSITY_SPAN,
        offset: f("map_offset", 0.0),
        direction: [cs, sn],
        aspect: [f("aspect_x", 1.0), f("aspect_y", 1.0)],
        tile: [f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5],
    }
}

pub(crate) fn resident_directional_warp(n: &Node) -> Option<crate::accelerator::DirectionalWarpParams> {
    let settings = directional_warp_params(n);
    let crate::accelerator::DirectionalWarpParams { intensity, offset, direction, aspect, .. } = settings;
    ([intensity, offset].into_iter().chain(direction).all(f32::is_finite)
        && aspect.into_iter().all(|axis| axis.is_finite() && intensity.abs() * (1.0 + offset.abs()) * axis.abs() <= 8192.0))
    .then_some(settings)
}

/// A `dir_warp` node: image `t` of pixels `P` moved along a direction by the luminance of `map`.
pub(crate) fn dir_warp<P: Pixel, M: crate::tex::Texels>(n: &Node, t: &Image<P>, map: &M) -> Image<P> {
    let crate::accelerator::DirectionalWarpParams { intensity, offset, direction: [cs, sn], aspect: [ax, ay], tile: [tx, ty] } =
        directional_warp_params(n);
    Image::from_fn(t.w, t.h, |u, v| {
        let d = intensity * (crate::tex::lum(map.sample(u, v).rgba()) - offset);
        sample_edge_stepped(t, u + d * cs * ax, v + d * sn * ay, tx, ty)
    })
}

/// A `grad_warp` node: image `t` of pixels `P` moved along the slopes of the luminance of `map`, to `size`.
pub(crate) fn grad_warp<P: Pixel, M: crate::tex::Texels>(n: &Node, t: &Image<P>, map: &M, size: [usize; 2]) -> Image<P> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let k = f("intensity", GRAD_WARP_DEFAULT_INTENSITY) / INTENSITY_SPAN;
    // the aspect scales the move along each axis (measured: mg_dripping_rust, 0.4967, mean 2.6 to 0.007)
    let (ax, ay) = (f("aspect_x", 1.0), f("aspect_y", 1.0));
    let (mw, mh) = (map.width(), map.height());
    let (tx, ty) = (f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5);
    let at = |x: usize, y: usize| crate::tex::lum(map.texel(y * mw + x).rgba());
    // slopes in uv units on the map's own pixels, from the pixel before (backward differences, wrapped). A result of
    // another size reads them bilinearly (measured: mg_dripping_rust, a 512 map under a 256 warp, mean 9.8 to 0.06).
    // An axis that doesn't repeat keeps its edge pixel there, so its first row or column has no slope, and the read
    // keeps the input's edge value (measured: ornament_leaf, whose map is bright on its left edge only: a wrapped
    // slope moved the leaf into the first column, which a later clamped read drew as stripes across the tile).
    let before = |x: usize, n: usize, wrap: bool| if wrap { (x + n - 1) % n } else { x.saturating_sub(1) };
    let slopes = Tex::from_fn(mw, mh, |u, v| {
        let (x, y) = (((u * mw as f32) as usize).min(mw - 1), ((v * mh as f32) as usize).min(mh - 1));
        let gx = (at(x, y) - at(before(x, mw, tx), y)) * mw as f32;
        let gy = (at(x, y) - at(x, before(y, mh, ty))) * mh as f32;
        [gx, gy, 0.0, 0.0]
    });
    let [w, h] = size;
    Image::from_fn(w, h, |u, v| {
        let g = slopes.sample(u, v);
        if tx && ty {
            sample_stepped(t, u + k * ax * g[0], v + k * ay * g[1])
        } else {
            sample_edge_stepped(t, u + k * ax * g[0], v + k * ay * g[1], tx, ty)
        }
    })
}

/// The background colour of a `transform_matrix` node: what it fades to past an edge that doesn't repeat.
pub(crate) fn transform_matrix_bg(n: &Node) -> [f32; 4] {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    [f("bg_r", 0.0), f("bg_g", 0.0), f("bg_b", 0.0), f("bg_a", 0.0)]
}

fn transform_matrix_coordinates(n: &Node) -> ([f32; 4], [f32; 2]) {
    let value = |name, default| n.nums.get(name).copied().unwrap_or(default);
    let matrix = [value("m00", 1.0), value("m01", 0.0), value("m10", 0.0), value("m11", 1.0)];
    let offset = [value("offset_x", 0.0), value("offset_y", 0.0)];
    (matrix, offset)
}

/// A wrapped bilinear affine transform at the source resolution of a square image.
pub(crate) fn resident_wrapped_color_affine(n: &Node) -> Option<([f32; 4], [f32; 2])> {
    let value = |name, default| n.nums.get(name).copied().unwrap_or(default);
    let (matrix, offset) = transform_matrix_coordinates(n);
    let mip = value("mip", MIP_AUTOMATIC as f32).round() as u32;
    (["tile_x", "tile_y", "filter"].into_iter().all(|key| value(key, 1.0) > 0.5)
        && transform_matrix_bg(n) == [0.0; 4]
        && matrix.into_iter().chain(offset).all(|coordinate| (-16.0..=16.0).contains(&coordinate))
        && transform_mip(mip, matrix) == 0)
        .then_some((matrix, offset))
}

/// On each non-repeating axis the extreme output centers must have full coverage.
fn affine_covers_image(matrix: [f32; 4], offset: [f32; 2], size: [usize; 2], tile: [bool; 2], fade: Option<[usize; 2]>) -> bool {
    let [m00, m01, m10, m11] = matrix;
    let [x, y] = size.map(|axis| 0.5 / axis as f32);
    [x, 1.0 - x].into_iter().all(|x| {
        [y, 1.0 - y].into_iter().all(|y| {
            let [x, y] = [x - 0.5, y - 0.5];
            let uv = [m00 * x + m10 * y + 0.5 + offset[0], m01 * x + m11 * y + 0.5 + offset[1]];
            uv.into_iter().zip(tile).zip(fade.unwrap_or_default()).all(|((coord, repeats), pixels)| {
                repeats || if fade.is_some() { coord.min(1.0 - coord) * pixels as f32 + 0.5 >= 1.0 } else { (0.0..1.0).contains(&coord) }
            })
        })
    })
}

/// The alpha a `transform_matrix` gives a grey image of alpha `alpha` (1 or +0), when its result is grey with one alpha
/// of 1 or +0 bit for bit: its background is grey, and the alpha it computes is the same everywhere. A bilinear read
/// gives `bg + (alpha - bg) * k` for the coverage `k` (always 1 where both axes repeat), a nearest one `alpha`, or `bg`
/// past an edge that doesn't repeat.
pub(crate) fn transform_grey_alpha(n: &Node, alpha: f32) -> Option<f32> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let bg = transform_matrix_bg(n);
    let (tx, ty, bilinear) = (f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5, f("filter", 1.0) > 0.5);
    if bg[0].to_bits() != bg[1].to_bits() || bg[1].to_bits() != bg[2].to_bits() {
        return None;
    }
    let out = if tx && ty {
        if bilinear { bg[3] + (alpha - bg[3]) * 1.0 } else { alpha }
    } else if bg[3].to_bits() == alpha.to_bits() {
        alpha
    } else {
        return None;
    };
    crate::tex::grey_alpha_ok(out).then_some(out)
}

/// A `transform_matrix` node on an image of pixels `P` (RGBA, or grey when [`transform_keeps_grey`]), `bg` its background.
pub(crate) fn transform_matrix<P: Pixel, E: crate::tex::Emit<P>>(n: &Node, t: &Image<P>, size: [usize; 2], bg: P, out: E) -> E::Out {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let (m, [ox, oy]) = transform_matrix_coordinates(n);
    let (tx, ty) = (f("tile_x", 1.0) > 0.5, f("tile_y", 1.0) > 0.5);
    let (bilinear, mip) = (f("filter", 1.0) > 0.5, f("mip", MIP_AUTOMATIC as f32).round() as u32);
    let [w, h] = size;
    // a size that changes unequally per axis: each axis reduces on its own by box, where one pyramid level for both would blur the axis
    // that keeps its size (measured: 0.002 at most on 38 of 38 such nodes, ours 0.44 to 2)
    let per_axis;
    let t = if bilinear && mip >= MIP_AUTOMATIC_TOO && ox == 0.0 && oy == 0.0 && is_axis_swap(m) {
        let (tw, th) = if m[0] == 0.0 { (h, w) } else { (w, h) };
        if (t.w > tw || t.h > th) && t.w * th != t.h * tw {
            let _span = prof("transform_matrix.per_axis");
            per_axis = reduced_per_axis(t, tw, th);
            &per_axis
        } else {
            t
        }
    } else {
        t
    };
    // a transform that changes nothing but the size enlarges by doubling, not with one bilinear read; with nearest
    // filtering it just repeats pixels (measured: mean 0.00 on every enlarging filter=0 node, about 30 with doubling)
    let doubled;
    let t = if bilinear && mip == MIP_AUTOMATIC && ox == 0.0 && oy == 0.0 && (t.w < w || t.h < h) && is_axis_swap(m) {
        let (w, h) = if m[0] == 0.0 { (h, w) } else { (w, h) };
        let _span = prof("transform_matrix.doubling");
        doubled = doubled_to(t, w, h, tx, ty, f("ofmt", 0.0).round() as u32);
        &doubled
    } else {
        t
    };
    // the pyramid goes by input pixels per output pixel, and the input can have another size than the result
    let (iw, ih, ow, oh) = (t.w as f32, t.h as f32, w.max(1) as f32, h.max(1) as f32);
    let scaled = [m[0] * iw / ow, m[1] * ih / ow, m[2] * iw / oh, m[3] * ih / oh];
    let level = transform_mip(mip, scaled);
    let reduced = {
        let _span = prof("transform_matrix.pyramid");
        crate::patterns::mip_of(t, level)
    };
    let s = reduced.as_ref().unwrap_or(t);
    let _span = prof("transform_matrix.reads");
    // a level chosen by the node itself fades out at a non-repeating edge over one of its own pixels, not one of the result's
    // (measured: an 8 x 8 white squeezed by 3 at level 3, rows 1/16, 7/16, 13/16 as a 1 x 1 level read bilinearly over black gives)
    // when the transform reduces (grunge_scratches_rough's node 71 too); one that enlarges keeps the result's pixel
    // (baked_lighting_material's 8 x 8 enlarged to 512 at level 1: 5.7 steps from the reference renderer with the level's pixel, 0.11
    // with the result's)
    let reduces = math::hypot(scaled[0], scaled[1]).max(math::hypot(scaled[2], scaled[3])) > 1.0;
    let fade = if mip < MIP_AUTOMATIC_TOO && level > 0 && reduces { [0, 0] } else { [w, h] };
    out.emit(w, h, |u, v| {
        let (x, y) = (u - 0.5, v - 0.5);
        let (su, sv) = (m[0] * x + m[2] * y + 0.5 + ox, m[1] * x + m[3] * y + 0.5 + oy);
        if bilinear { sample_bilinear(s, su, sv, tx, ty, bg, fade) } else { sample_nearest(s, su, sv, tx, ty, bg) }
    })
}

/// A `levels_mid` node's settings per channel (low, high, mid, out low, out high for r, g, b, a), and whether it holds
/// floats.
fn levels_params(n: &Node) -> ([[f32; 5]; 4], bool) {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let ch = |k: &str, c: &str, d: f32| n.nums.get(&format!("{k}_{c}")).copied().unwrap_or_else(|| f(k, d));
    let params: [[f32; 5]; 4] =
        ["r", "g", "b", "a"].map(|c| [ch("low", c, 0.0), ch("high", c, 1.0), ch("mid", c, 0.5), ch("out_low", c, 0.0), ch("out_high", c, 1.0)]);
    // a node holding floats keeps output bounds outside 0..1 (measured: bandage #46 and fabric_tear #167, single floats with
    // bounds -0.15..0.15 and -0.2..0.2, go below zero: mean 9.8 and 19.5 to 0); integer nodes limit them first
    let float = !matches!(n.nums.get("ofmt").map(|v| v.round() as u32), Some(COMPILED_8_BIT | COMPILED_16_BIT));
    (params, float)
}

/// `levels_mid` on a grey image, when its result is grey: the same settings for red, green and blue, and an alpha that
/// comes out 1 or +0. The result and its alpha.
pub(crate) fn levels_mid_grey(n: &Node, t: crate::tex::GreyIn) -> Option<(crate::tex::Grey, f32)> {
    let (params, float) = levels_params(n);
    let level = move |v: f32, q: &[f32; 5]| if float { levels_value_unbounded(v, q) } else { levels_value(v, q) };
    let [r, g, b, a] = params;
    let same = |x: &[f32; 5], y: &[f32; 5]| x.iter().zip(y).all(|(x, y)| x.to_bits() == y.to_bits());
    if !same(&r, &g) || !same(&g, &b) {
        return None;
    }
    let alpha = level(t.alpha(), &a);
    if !crate::tex::grey_alpha_ok(alpha) {
        return None;
    }
    Some((t.floats().map(move |v| level(v, &r)), alpha))
}

/// Exact output steps for every input step of a packed grey levels node.
pub(crate) fn levels_mid_curve(n: &Node, div: f32, alpha: f32, steps: &crate::tex::Steps) -> Option<(Vec<u16>, f32)> {
    let ([r, g, b, a], float) = levels_params(n);
    if !matches!(div, 255.0 | 65535.0)
        || r.map(f32::to_bits) != g.map(f32::to_bits)
        || r.map(f32::to_bits) != b.map(f32::to_bits)
        || ![r, a].iter().flatten().all(|value| value.is_finite())
    {
        return None;
    }
    let level = if float { levels_value_unbounded } else { levels_value };
    let alpha = level(alpha, &a);
    if !crate::tex::grey_alpha_ok(alpha) {
        return None;
    }
    let curve = (0..=div as u16).map(|i| steps.step(level(f32::from(i) / div, &r))).collect::<Option<_>>()?;
    Some((curve, alpha))
}

/// A matrix that only flips or swaps the axes.
fn is_axis_swap(m: [f32; 4]) -> bool {
    let unit = |a: f32, b: f32| (a.abs() == 1.0 && b == 0.0) || (a == 0.0 && b.abs() == 1.0);
    unit(m[0], m[1]) && unit(m[2], m[3]) && (m[0] == 0.0) == (m[3] == 0.0)
}

/// Halved along each axis longer than the size, an axis on its own, every pass held in 16-bit steps as a pyramid's level is
/// (measurement "What nodes hold and how they read": pyramid levels hold 16-bit steps per pass, P).
fn reduced_per_axis<P: Pixel>(t: &Image<P>, w: usize, h: usize) -> Image<P> {
    let mut o = t.clone();
    while o.w / 2 >= w.max(1) || o.h / 2 >= h.max(1) {
        let (nw, nh) = (if o.w / 2 >= w.max(1) { o.w / 2 } else { o.w }, if o.h / 2 >= h.max(1) { o.h / 2 } else { o.h });
        let mut n = Image::solid(nw, nh, P::NONE);
        let (fx, fy) = (o.w / nw, o.h / nh);
        for y in 0..nh {
            for x in 0..nw {
                let mut s = [0.0f64; 4];
                for j in 0..fy {
                    for i in 0..fx {
                        let p = o.px_at((y * fy + j) * o.w + x * fx + i);
                        s.iter_mut().enumerate().take(P::CH).for_each(|(c, a)| *a += f64::from(p.ch(c)));
                    }
                }
                n.set_px(y * nw + x, P::from_ch(|c| ((get(&s, c) / (fx * fy) as f64 * STEPS_16 + 0.5).floor() / STEPS_16) as f32));
            }
        }
        o = n;
    }
    o
}

/// Enlarged by doubling until it reaches the size: every new pixel 3/4 of the nearer pixel and 1/4 of the next. Measured:
/// exact to 0.002 of an 8-bit step, where a single bilinear read of a 32 times larger image is 5 steps off on average.
fn doubled_to<P: Pixel>(t: &Image<P>, w: usize, h: usize, tx: bool, ty: bool, format: u32) -> Image<P> {
    let mut o = t.clone();
    while o.w < w || o.h < h {
        if o.w < w {
            o = double_columns(&o, tx);
        }
        if o.h < h {
            o = double_rows(&o, ty);
        }
        // each doubling is a read of its own, held in the node's format
        if matches!(format, COMPILED_8_BIT | COMPILED_16_BIT) {
            crate::ops::round_to_16bit_read(&mut o);
            crate::ops::apply_format(&mut o, format);
        }
    }
    o
}

/// Twice the columns: [`double_rows`] across, without transposing the image there and back (the same values).
fn double_columns<P: Pixel>(t: &Image<P>, wrap: bool) -> Image<P> {
    let (w, h) = (t.w, t.h);
    let mut o = Image::solid(2 * w, h, P::NONE);
    let neighbour = |x: usize, left: bool| match (left, wrap) {
        (true, true) => (x + w - 1) % w,
        (true, false) => x.saturating_sub(1),
        (false, true) => (x + 1) % w,
        (false, false) => (x + 1).min(w - 1),
    };
    crate::par::for_each_run_mut(&mut o.px, 2 * w * 16, |start, chunk| {
        for (i, d) in chunk.iter_mut().enumerate() {
            let k = start + i;
            let (col, y) = (k % (2 * w), k / (2 * w));
            let (x, left) = (col / 2, col % 2 == 0);
            let (a, b) = (t.px_at(y * w + x), t.px_at(y * w + neighbour(x, left)));
            *d = a.zip(b, |a, b| DOUBLING_NEAR * a + DOUBLING_FAR * b);
        }
    });
    o
}

/// Twice the rows.
fn double_rows<P: Pixel>(t: &Image<P>, wrap: bool) -> Image<P> {
    let (w, h) = (t.w, t.h);
    let mut o = Image::solid(w, 2 * h, P::NONE);
    let neighbour = |y: usize, up: bool| match (up, wrap) {
        (true, true) => (y + h - 1) % h,
        (true, false) => y.saturating_sub(1),
        (false, true) => (y + 1) % h,
        (false, false) => (y + 1).min(h - 1),
    };
    crate::tex::par_rows(&mut o.px, w, |start, chunk| {
        for (i, d) in chunk.iter_mut().enumerate() {
            let k = start + i;
            let (x, row) = (k % w, k / w);
            let (y, up) = (row / 2, row % 2 == 0);
            let (a, b) = (t.px_at(y * w + x), t.px_at(neighbour(y, up) * w + x));
            *d = a.zip(b, |a, b| DOUBLING_NEAR * a + DOUBLING_FAR * b);
        }
    });
    o
}

/// The pyramid level a transform reads: `mip` itself, or for 31 the rounded log2 of how many input pixels one output
/// pixel spans along the longer axis of the matrix (given here in input pixels per output pixel).
fn transform_mip(mip: u32, m: [f32; 4]) -> u32 {
    // 31 goes by the scale, and so does 30 (measured)
    if mip < MIP_AUTOMATIC_TOO {
        return mip;
    }
    crate::patterns::mip_level(math::hypot(m[0], m[1]).max(math::hypot(m[2], m[3])))
}

/// A pixel of `t`; on an axis that does not repeat, `bg` outside the image.
fn border_px<P: Pixel>(t: &Image<P>, x: i64, y: i64, tx: bool, ty: bool, bg: P) -> P {
    if (!tx && !(0..t.w as i64).contains(&x)) || (!ty && !(0..t.h as i64).contains(&y)) {
        return bg;
    }
    t.at(x, y)
}

fn sample_nearest<P: Pixel>(t: &Image<P>, u: f32, v: f32, tx: bool, ty: bool, bg: P) -> P {
    border_px(t, (u * t.w as f32).floor() as i64, (v * t.h as f32).floor() as i64, tx, ty, bg)
}

/// A bilinear read. On an axis that does not repeat the colour keeps its edge value and fades to `bg` across the edge,
/// over one pixel of the input, or of the result when the input is smaller than the result (measured: at the same size
/// the edge pixels mix with `bg`, while a 16 pixel wide input enlarged to 512 doesn't fade over a whole input pixel).
fn sample_bilinear<P: Pixel>(t: &Image<P>, u: f32, v: f32, tx: bool, ty: bool, bg: P, result: [usize; 2]) -> P {
    let axis = |a: f32, n: usize, out: usize, wrap: bool| -> (f32, f32) {
        if wrap {
            return (a, 1.0);
        }
        let half = 0.5 / n as f32;
        (a.clamp(half, 1.0 - half), (a.min(1.0 - a) * n.max(out) as f32 + 0.5).clamp(0.0, 1.0))
    };
    let (su, cx) = axis(u, t.w, result[0], tx);
    let (sv, cy) = axis(v, t.h, result[1], ty);
    let c = sample_stepped(t, su, sv);
    let k = cx * cy;
    bg.zip(c, |bg, c| bg + (c - bg) * k)
}

/// A bilinear read with the weights in steps of 1/256, as texture hardware has them. Wrapped. Measurement "Transforms" and "What
/// nodes hold and how they read"; the rule table `bilinear_read.tsv`.
pub(crate) fn sample_stepped<P: Pixel>(t: &Image<P>, u: f32, v: f32) -> P {
    stepped_with(t, u, v, |i, j| t.at(i, j))
}

/// [`sample_stepped`] with transparent black past the image's edges instead of its wrap: how a fx-map reads an image pattern
/// (measured: instances of a pixel or so, stitches' strips and plastic_grip's dots, came out a half to a third as bright as a
/// wrapped read gives, mean 51 and 35 off; with the edge fading to black 0.17 and 0.12, 100% within one step).
pub(crate) fn sample_stepped_bordered(t: &Tex, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (t.w as i64, t.h as i64);
    stepped_with(t, u, v, |i, j| if (0..w).contains(&i) && (0..h).contains(&j) { t.px_at((j * w + i) as usize) } else { [0.0; 4] })
}

fn stepped_with<P: Pixel>(t: &Image<P>, u: f32, v: f32, at: impl Fn(i64, i64) -> P) -> P {
    let (x, y) = (f64::from(u) * t.w as f64 - 0.5, f64::from(v) * t.h as f64 - 0.5);
    let (x0, y0) = (crate::tex::floor64(x), crate::tex::floor64(y));
    // round() without a libm call: from 0.5 on, adding a half is exact or can't cross an integer, so the floor of it is
    // the rounding; below that (and anything odd) round() itself
    let step = |f: f64| {
        let s = f * WEIGHT_STEPS;
        let r = if (0.5..WEIGHT_STEPS).contains(&s) { crate::tex::floor64(s + 0.5) } else { s.round() };
        (r / WEIGHT_STEPS) as f32
    };
    let (fx, fy) = (step(x - x0), step(y - y0));
    // (an infinite coordinate saturates to the end of the range: the next pixel wraps round, as release builds always did)
    let (i, j) = (x0 as i64, y0 as i64);
    let (i1, j1) = (i.wrapping_add(1), j.wrapping_add(1));
    let (a, b, c, d) = (at(i, j), at(i1, j), at(i, j1), at(i1, j1));
    let (top, bottom) = (a.zip(b, |a, b| a + (b - a) * fx), c.zip(d, |c, d| c + (d - c) * fx));
    top.zip(bottom, |top, bottom| top + (bottom - top) * fy)
}

/// Each row averaged over a box reaching `r` pixels from every pixel's centre, on the row as a stepped function; beyond
/// the ends the row repeats or keeps its edge value.
/// A blur on a node holding 8 or 16 bits or half floats: across, rounded to that, then down.
fn held_blur<P: Pixel>(t: &Image<P>, rx: f32, ry: f32, tx: bool, ty: bool, format: u32) -> Image<P> {
    let mut across = compiled_box_axis(t, rx, tx, format);
    crate::ops::apply_format(&mut across, format);
    transposed(&compiled_box_axis(&transposed(&across), ry, ty, format))
}

/// A box along rows as compiled graphs compute it on an 8- or 16-bit node. Measured at 16 bits (`scripts/blur16w.py`, impulses and a rich
/// image; 100% of pixels exact to width 7, 86 to 98% to width 41, the rest one 16-bit step off):
/// - reads are bilinear with the weight in steps of 1/256, rounded to a 16-bit step, half up;
/// - to width 5: the centre, and on each side one read of the two pixels there, weighted by what they cover;
/// - wider: one end of the window is read alone, the other as a read like those, and what's between comes from a pyramid of the row: each
///   level the pairs of the one before, aligned, held in the node's format (16 bits: half up). Two texels of a level that don't make up
///   one of the next are read together without rounding. Of an odd number the one that doesn't line up goes alone: the first if it's odd, else the last.
///
/// The result is the sum over the width, unrounded; the node's format rounds it. Other formats and rows that don't tile keep [`box_axis`].
/// Measurement "What nodes hold and how they read" ("Box blur along an axis on an integer node"); the rule table `box_blur_plan.tsv`.
pub(crate) fn compiled_box_axis<P: Pixel>(t: &Image<P>, r: f32, wrap: bool, format: u32) -> Image<P> {
    let r = f64::from(r.abs());
    if r <= 0.5 {
        return t.clone();
    }
    let h = (r - 0.5).ceil() as i64;
    let edge = r - (h as f64 - 0.5);
    let n = 2 * h + 1;
    if !wrap || !matches!(format, COMPILED_8_BIT | COMPILED_16_BIT) || !t.w.is_power_of_two() || n >= t.w as i64 {
        return box_axis(t, r as f32, wrap);
    }
    let w = t.w;
    let hold = move |v: f64| -> f64 {
        if format == COMPILED_16_BIT {
            v.floor() + if v.fract() >= 0.5 { 1.0 } else { 0.0 }
        } else {
            f64::from(crate::ops::compiled_8bit((v / STEPS_16) as f32)) * STEPS_16
        }
    };
    // the second pixel's weight in a read of an inner pixel and the window's end
    let far = ((edge / (1.0 + edge) * WEIGHT_STEPS).round()) / WEIGHT_STEPS;
    let top = if n > SHORT_BOX_WIDTH { math::log2((n - 1) as f64).ceil() as usize - 1 } else { 0 };
    let mut out = t.clone();
    crate::tex::par_rows(&mut out.px, w, |start, chunk| {
        // (the chunks are whole rows from row `start / w` on)
        for (dst, row) in chunk.chunks_mut(w).zip(t.px.chunks(w.max(1)).skip(start / w.max(1))) {
            for ch in 0..P::CH {
                // (a whole step in single precision comes back up to 0.004 off it)
                let units = |v: f32| {
                    let u = f64::from(v) * STEPS_16;
                    if (u - u.round()).abs() < WHOLE_STEP_SLACK { u.round() } else { u }
                };
                let mut levels: Vec<Vec<f64>> = vec![row.iter().map(|p| units(p.ch(ch))).collect()];
                for l in 0..top {
                    let Some(below) = levels.get(l) else { break };
                    if below.len() < 2 {
                        break;
                    }
                    // (a level of odd length has no pair for its last texel: it is left out)
                    let next = below.as_chunks::<2>().0.iter().map(|[a, b]| hold((a + b) / 2.0)).collect();
                    levels.push(next);
                }
                let base = levels.first().map_or(&[][..], Vec::as_slice);
                let at = |x: i64| get(base, x.rem_euclid((w as i64).max(1)) as usize);
                let read = |near: i64, end: i64| ((1.0 - far) * at(near) + far * at(end) + 0.5).floor();
                for (x, d) in dst.iter_mut().enumerate() {
                    let x = x as i64;
                    let sum = if n == 3 {
                        at(x) + edge * (at(x - 1) + at(x + 1))
                    } else if n == SHORT_BOX_WIDTH {
                        at(x) + (1.0 + edge) * (read(x - 1, x - 2) + read(x + 1, x + 2))
                    } else {
                        let (s, e) = (x - h, x + h);
                        // what's left after the end read alone lines up with the pairs
                        if s.rem_euclid(2) == 1 {
                            edge * at(s) + (1.0 + edge) * read(e - 1, e) + pyramid_sum(&levels, (s + 1).div_euclid(2), (e - 2).div_euclid(2), 1, top)
                        } else {
                            edge * at(e) + (1.0 + edge) * read(s + 1, s) + pyramid_sum(&levels, (s + 2).div_euclid(2), (e - 1).div_euclid(2), 1, top)
                        }
                    };
                    let mean = sum / (2.0 * r);
                    // to 16 bits here, in double precision: single precision moves values across the rounding
                    let held = if format == COMPILED_16_BIT {
                        ((STEPS_16 * (mean / STEPS_16 * FINE_16 as f64).floor() + FINE_16_TIE_DOWN as f64) / FINE_16 as f64).floor()
                    } else {
                        mean
                    };
                    let v = (held / STEPS_16) as f32;
                    *d = P::from_ch(|c| if c == ch { v } else { d.ch(c) });
                }
            }
        }
    });
    out
}

/// The sum over texels `s..=e` of level `l` of a row's pyramid, in pixels of the row. See [`compiled_box_axis`].
fn pyramid_sum(levels: &[Vec<f64>], s: i64, e: i64, l: usize, top: usize) -> f64 {
    let n = e - s + 1;
    if n <= 0 {
        return 0.0;
    }
    let weight = (1u64 << l) as f64;
    let level = levels.get(l).map_or(&[][..], Vec::as_slice);
    let at = |i: i64| get(level, i.rem_euclid((level.len() as i64).max(1)) as usize);
    if n == 1 {
        return weight * at(s);
    }
    if l >= top || l + 1 >= levels.len() {
        return weight * (s..=e).map(at).sum::<f64>();
    }
    if n == 2 && s.rem_euclid(2) == 1 {
        return weight * (at(s) + at(e));
    }
    if n % 2 == 1 {
        return if s.rem_euclid(2) == 1 {
            weight * at(s) + pyramid_sum(levels, s + 1, e, l, top)
        } else {
            weight * at(e) + pyramid_sum(levels, s, e - 1, l, top)
        };
    }
    if s.rem_euclid(2) == 0 {
        return pyramid_sum(levels, s.div_euclid(2), (e - 1).div_euclid(2), l + 1, top);
    }
    weight * (at(s) + at(e)) + pyramid_sum(levels, (s + 1).div_euclid(2), (e - 2).div_euclid(2), l + 1, top)
}

fn box_axis<P: Pixel>(t: &Image<P>, r: f32, wrap: bool) -> Image<P> {
    let (w, r) = (t.w, f64::from(r.abs()));
    if r <= 0.5 {
        return t.clone();
    }
    let mut out = t.clone();
    crate::tex::par_rows(&mut out.px, w, |start, chunk| {
        let mut cum = vec![[0.0f64; 4]; w + 1];
        // (the chunks are whole rows from row `start / w` on)
        for (dst, row) in chunk.chunks_mut(w).zip(t.px.chunks(w.max(1)).skip(start / w.max(1))) {
            for (x, p) in row.iter().enumerate() {
                let before = get(&cum, x);
                if let Some(next) = cum.get_mut(x + 1) {
                    *next = std::array::from_fn(|c| get(&before, c) + f64::from(p.ch(c)));
                }
            }
            let total = get(&cum, w);
            let n = w as f64;
            // the integral of the row up to position x
            let integral = |x: f64, c: usize| -> f64 {
                let (cycles, x, whole) = if wrap {
                    let k = (x / n).floor();
                    (k, x - k * n, 0.0)
                } else if x < 0.0 {
                    return f64::from(row.first().map_or(0.0, |p| p.ch(c))) * x;
                } else if x >= n {
                    return get(&total, c) + f64::from(row.get(w.saturating_sub(1)).map_or(0.0, |p| p.ch(c))) * (x - n);
                } else {
                    (0.0, x, 0.0)
                };
                let i = (x.floor() as usize).min(w.saturating_sub(1));
                whole + cycles * get(&total, c) + get(&get(&cum, i), c) + f64::from(row.get(i).map_or(0.0, |p| p.ch(c))) * (x - i as f64)
            };
            for (x, d) in dst.iter_mut().enumerate() {
                let centre = x as f64 + 0.5;
                *d = P::from_ch(|c| ((integral(centre + r, c) - integral(centre - r, c)) / (2.0 * r)) as f32);
            }
        }
    });
    out
}

/// A profile span of `name` when profiling is on (see `crate::profile`); nothing otherwise.
fn prof(name: &str) -> Option<crate::profile::Span> {
    crate::profile::enabled().then(|| crate::profile::span(name))
}

fn transposed<P: Pixel>(t: &Image<P>) -> Image<P> {
    let mut px = vec![P::NONE; t.px.len()];
    // whole result rows (input columns) to each thread
    let h = t.h.max(1);
    crate::par::for_each_run_mut(&mut px, h * 16, |start, chunk| {
        for (i, d) in chunk.iter_mut().enumerate() {
            let k = start + i;
            *d = t.px_at((k % h) * t.w + k / h);
        }
    });
    Image { w: t.h, h: t.w, px }
}

fn box_columns<P: Pixel>(t: &Image<P>, r: f32, wrap: bool) -> Image<P> {
    transposed(&box_axis(&transposed(t), r, wrap))
}

/// Blur along a line of half-length `intensity` pixels of a 256 pixel image, per axis scaled by the image's size and the
/// aspect. Measured on impulses (exact to a fraction of a 16-bit step): `v = ((rx - 0.5) cos, (ry - 0.5) sin)` is the line,
/// with x the major axis when `|vx| / w >= |vy| / h`. Below one pixel of length a pixel mixes with two reads one step along
/// the major axis (`line_blur_direct`). From there on the image is sheared along the minor axis, each major-axis line
/// averaged by a box of half-width `0.5 + |v major|`, and sheared back (`line_blur_sheared`).
fn line_blur<P: Pixel>(t: &Image<P>, intensity: f32, aspect: [f32; 2], angle: f32, tx: bool, ty: bool) -> Option<Image<P>> {
    let (s, c) = math::sin_cos(angle * std::f32::consts::TAU);
    let (rx, ry) = (intensity * aspect[0] * t.w as f32 / INTENSITY_SPAN, intensity * aspect[1] * t.h as f32 / INTENSITY_SPAN);
    if s.abs() < AXIS_SLACK {
        return Some(box_axis(t, rx, tx));
    }
    if c.abs() < AXIS_SLACK {
        return Some(box_columns(t, ry, ty));
    }
    let (dx, dy) = ((rx - LINE_HALF_PIXEL).max(0.0) * c, (ry - LINE_HALF_PIXEL).max(0.0) * s);
    let longest = dx.abs().max(dy.abs());
    if longest <= 0.0 {
        return Some(t.clone());
    }
    // a tie (45 degrees) goes to y
    let along_y = f64::from(dx.abs()) * t.h as f64 <= f64::from(dy.abs()) * t.w as f64 * (1.0 + TIE_SLACK);
    if along_y {
        let n = transposed(t);
        let o = if longest < 1.0 {
            line_blur_direct(&n, dx / dy * t.h as f32 / t.w as f32, LINE_HALF_PIXEL + longest, ty, tx)
        } else {
            line_blur_sheared(&n, dx / dy, LINE_HALF_PIXEL + dy.abs(), ty, tx)?
        };
        Some(transposed(&o))
    } else if longest < 1.0 {
        Some(line_blur_direct(t, dy / dx * t.w as f32 / t.h as f32, LINE_HALF_PIXEL + longest, tx, ty))
    } else {
        line_blur_sheared(t, dy / dx, LINE_HALF_PIXEL + dx.abs(), tx, ty)
    }
}

/// Position `p` along a column as the two rows it falls between and the weight of the second: the weight in steps of 1/256,
/// as texture hardware has it.
fn shear_lerp(p: f64) -> (i64, f32) {
    let i = p.floor();
    (i as i64, (((p - i) * WEIGHT_STEPS).round() / WEIGHT_STEPS) as f32)
}

/// Bilinear along a column.
fn lerp_rows<P: Pixel>(a: P, b: P, k: f32) -> P {
    a.zip(b, |a, b| a + (b - a) * k)
}

/// A short line: each pixel with the reads one column to either side, `slope` rows away, with weights that make a box of
/// half-width `r` (at most the pixel itself when `r` is 0.5).
fn line_blur_direct<P: Pixel>(t: &Image<P>, slope: f32, r: f32, tx: bool, ty: bool) -> Image<P> {
    let (w, h) = (t.w as i64, t.h as i64);
    let row = |y: i64| if ty { y.rem_euclid(h) } else { y.clamp(0, h - 1) };
    let col = |x: i64| if tx { x.rem_euclid(w) } else { x.clamp(0, w - 1) };
    let read = |x: i64, y: i64, off: f64| {
        let (i, k) = shear_lerp(y as f64 + off);
        let x = col(x);
        lerp_rows(t.px_at((row(i) * w + x) as usize), t.px_at((row(i + 1) * w + x) as usize), k)
    };
    let side = (r - 0.5).max(0.0);
    let mut out = t.clone();
    crate::tex::par_rows(&mut out.px, t.w, |start, chunk| {
        for (i, d) in chunk.iter_mut().enumerate() {
            let k = (start + i) as i64;
            let (x, y) = (k % w, k / w);
            let (l, m, n) = (read(x - 1, y, -f64::from(slope)), t.px_at(k as usize), read(x + 1, y, f64::from(slope)));
            *d = m.zip(l.zip(n, |l, n| l + n), |m, ln| (m + side * ln) / (2.0 * r));
        }
    });
    out
}

/// Shear, box, shear back. The shear is `slope` rows per column around the image's centre. Rows repeat or keep their edge.
/// Past a side that doesn't repeat each row keeps its last value, in the sheared image. Past a side that repeats the shear
/// goes on, so that a line crossing the seam stays straight.
fn line_blur_sheared<P: Pixel>(t: &Image<P>, slope: f32, r: f32, tx: bool, ty: bool) -> Option<Image<P>> {
    let (w, h) = (t.w as i64, t.h as i64);
    let (r64, slope) = (f64::from(r), f64::from(slope));
    let pad = r64.ceil() as i64 + 2;
    let shift = |x: i64| (x as f64 + 0.5 - w as f64 / 2.0) * slope;
    // rows past the image's top and bottom hold what shears out of it, when the rows don't repeat
    let extra = if ty { 0 } else { (slope.abs() * (w as f64 / 2.0 + pad as f64 + r64)) as i64 + 3 };
    let row = |y: i64| if ty { y.rem_euclid(h) } else { y.clamp(0, h - 1) };
    let rows = (h + 2 * extra) as usize;
    let span = (w + 2 * pad) as usize;
    if rows > MAX_SHEARED_SIDE || span > MAX_SHEARED_SIDE {
        return None;
    }
    // the sheared image, averaged along its rows; columns `0..w` of it are kept
    let blurred: Vec<Vec<P>> = crate::tex::par_map(rows, |ry| {
        let y = ry as i64 - extra;
        let mut cum = vec![[0.0f64; 4]; span + 1];
        for k in 0..span {
            let at = k as i64 - pad;
            let x = if tx { at } else { at.clamp(0, w - 1) };
            let (i, f) = shear_lerp(y as f64 + shift(x));
            let cx = x.rem_euclid(w);
            let p = lerp_rows(t.px_at((row(i) * w + cx) as usize), t.px_at((row(i + 1) * w + cx) as usize), f);
            let before = get(&cum, k);
            if let Some(next) = cum.get_mut(k + 1) {
                *next = std::array::from_fn(|c| get(&before, c) + f64::from(p.ch(c)));
            }
        }
        // the integral of the row, a stepped function, up to a position inside it
        let integral = |x: f64, c: usize| {
            let x = x.clamp(0.0, span as f64);
            let i = (x.floor() as usize).min(span.saturating_sub(1));
            let (a, b) = (get(&get(&cum, i), c), get(&get(&cum, i + 1), c));
            a + (b - a) * (x - i as f64)
        };
        (0..t.w)
            .map(|x| {
                let centre = (x as i64 + pad) as f64 + 0.5;
                P::from_ch(|c| ((integral(centre + r64, c) - integral(centre - r64, c)) / (2.0 * r64)) as f32)
            })
            .collect()
    });
    let mut out = t.clone();
    crate::tex::par_rows(&mut out.px, t.w, |start, chunk| {
        for (i, d) in chunk.iter_mut().enumerate() {
            let k = (start + i) as i64;
            let (x, y) = (k % w, k / w);
            let (j, f) = shear_lerp(y as f64 - shift(x));
            let at = |q: i64| if ty { q.rem_euclid(h) as usize } else { (q + extra).clamp(0, rows as i64 - 1) as usize };
            let read = |row: usize| blurred.get(row).and_then(|r| r.get(x as usize).copied()).unwrap_or(P::NONE);
            *d = lerp_rows(read(at(j)), read(at(j + 1)), f);
        }
    });
    Some(out)
}

/// The longest side of the sheared image a line blur works on: a line far longer than the image (a huge intensity) is
/// refused instead of taking memory without end (a guard, not measured).
const MAX_SHEARED_SIDE: usize = 1 << 15;

/// Bilinear read; an axis that does not repeat keeps its edge value.
fn sample_edge<P: Pixel>(t: &Image<P>, u: f32, v: f32, tx: bool, ty: bool) -> P {
    let lim = |a: f32, n: usize, wrap: bool| if wrap { a } else { a.clamp(0.5 / n as f32, 1.0 - 0.5 / n as f32) };
    t.sample(lim(u, t.w, tx), lim(v, t.h, ty))
}

/// [`sample_edge`] with the weights in steps of 1/256.
fn sample_edge_stepped<P: Pixel>(t: &Image<P>, u: f32, v: f32, tx: bool, ty: bool) -> P {
    let lim = |a: f32, n: usize, wrap: bool| if wrap { a } else { a.clamp(0.5 / n as f32, 1.0 - 0.5 / n as f32) };
    sample_stepped(t, lim(u, t.w, tx), lim(v, t.h, ty))
}

fn laplacian_sharpen(t: &Tex, intensity: [f32; 2], tx: bool, ty: bool) -> Tex {
    let (w, h) = (t.w as i64, t.h as i64);
    let at = |x: i64, y: i64| {
        let x = if tx { x } else { x.clamp(0, w - 1) };
        let y = if ty { y } else { y.clamp(0, h - 1) };
        t.at(x, y)
    };
    let mut out = t.clone();
    crate::tex::par_rows(&mut out.px, t.w, |start, chunk| {
        for (i, d) in chunk.iter_mut().enumerate() {
            let k = start + i;
            let (x, y) = ((k % t.w) as i64, (k / t.w) as i64);
            let (p, l, r, u, b) = (t.px_at(k), at(x - 1, y), at(x + 1, y), at(x, y - 1), at(x, y + 1));
            for (c, d) in d.iter_mut().enumerate() {
                let (p, l, r, u, b) = (get(&p, c), get(&l, c), get(&r, c), get(&u, c), get(&b, c));
                *d = (p + intensity[0] * (2.0 * p - l - r) + intensity[1] * (2.0 * p - u - b)).clamp(0.0, 1.0);
            }
        }
    });
    out
}

/// One channel through levels: `q` = [low, high, mid, out_low, out_high]. After the low/high stretch the value is raised
/// to a power that the midpoint sets: 1 + 16 |mid - 0.5|, inverted below the middle.
pub fn levels_value(v: f32, q: &[f32; 5]) -> f32 {
    let [lo, hi, mid, ol, oh] = *q;
    // an output bound outside 0..1 counts as the end of the range (measured: `light`, whose expressions reach -0.275 and 1.225)
    levels_value_unbounded(v, &[lo, hi, mid, ol.clamp(0.0, 1.0), oh.clamp(0.0, 1.0)])
}

/// [`levels_value`] with the output bounds as they are.
pub fn levels_value_unbounded(v: f32, q: &[f32; 5]) -> f32 {
    let [lo, hi, mid, ol, oh] = *q;
    let d = hi - lo;
    // low = high: a step, and a value right on it is below (measured at 0, 0.5 and 1)
    let t = if d.abs() < LEVELS_STEP_SLACK { if v > hi { 1.0 } else { 0.0 } } else { ((v - lo) / d).clamp(0.0, 1.0) };
    let k = 1.0 + LEVELS_MID_POWER * (mid - 0.5).abs();
    let gamma = if mid < 0.5 { 1.0 / k } else { k };
    ol + (oh - ol) * math::powf(t, gamma)
}

/// HSL shift: 0.5 changes nothing and a parameter's distance from it, doubled, is added: to the hue in turns, to the
/// saturation and to the lightness, the last two limited to 0..1.
pub fn hsl_shift(p: [f32; 4], h: f32, s: f32, l: f32) -> [f32; 4] {
    let (hh, ss, ll) = rgb_to_hsl(p[0], p[1], p[2]);
    let hh = (hh + (h - 0.5) * HSL_SHIFT_GAIN).rem_euclid(1.0);
    let ss = (ss + (s - 0.5) * HSL_SHIFT_GAIN).clamp(0.0, 1.0);
    let ll = (ll + (l - 0.5) * HSL_SHIFT_GAIN).clamp(0.0, 1.0);
    let (r, g, b) = hsl_to_rgb(hh, ss, ll);
    [r, g, b, p[3]]
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    if d < 1e-7 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s <= 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepped_reads_and_doubled_columns_are_what_they_replace() {
        // the bilinear read with libm's floor and round, as it was
        let old = |t: &Tex, u: f32, v: f32| -> [f32; 4] {
            let (x, y) = (f64::from(u) * t.w as f64 - 0.5, f64::from(v) * t.h as f64 - 0.5);
            let (x0, y0) = (x.floor(), y.floor());
            let step = |f: f64| ((f * 256.0).round() / 256.0) as f32;
            let (fx, fy) = (step(x - x0), step(y - y0));
            let (i, j) = (x0 as i64, y0 as i64);
            let (i1, j1) = (i.wrapping_add(1), j.wrapping_add(1));
            let (a, b, c, d) = (t.at(i, j), t.at(i1, j), t.at(i, j1), t.at(i1, j1));
            std::array::from_fn(|k| {
                let (top, bottom) = (a[k] + (b[k] - a[k]) * fx, c[k] + (d[k] - c[k]) * fx);
                top + (bottom - top) * fy
            })
        };
        let t = Tex::from_fn(37, 16, |u, v| [u, v, math::sin(u * 40.0), u * v - 0.5]);
        let mut x = 0x9e37_79b9u32;
        let mut coords = vec![0.0f32, -0.0, 0.5, 1.0, -1.0, 1e-30, f32::NAN, f32::INFINITY, 1e9, -1e9];
        for k in 0..=4096 {
            let a = k as f32 / 4096.0;
            coords.extend([a, a.next_up(), a.next_down(), a + 1.0 / 74.0, a - 1.0 / 32.0]);
        }
        for _ in 0..100_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            coords.push((x as f32 / u32::MAX as f32) * 6.0 - 3.0);
        }
        for (k, u) in coords.iter().enumerate() {
            let v = coords[(k * 7919) % coords.len()];
            let (a, b) = (sample_stepped(&t, *u, v), old(&t, *u, v));
            assert!(a.iter().zip(b).all(|(p, q)| p.to_bits() == q.to_bits()), "{u} {v}: {a:?} {b:?}");
        }
        // doubling across without a transpose there and back, both ways at the edge
        for wrap in [false, true] {
            let (a, b) = (double_columns(&t, wrap), transposed(&double_rows(&transposed(&t), wrap)));
            assert_eq!((a.w, a.h), (b.w, b.h));
            assert!(a.px.iter().flatten().zip(b.px.iter().flatten()).all(|(p, q)| p.to_bits() == q.to_bits()));
        }
    }

    #[test]
    fn levels_midpoint_sets_the_power() {
        // midpoint 0.25: power 1/5, midpoint 0.75: power 5
        let q = [0.0, 1.0, 0.25, 0.0, 1.0];
        assert!((levels_value(0.25, &q) - math::powf(0.25f32, 0.2)).abs() < 1e-5);
        assert!((levels_value(0.5, &[0.0, 1.0, 0.75, 0.0, 1.0]) - 0.03125).abs() < 1e-5);
        assert!((levels_value(0.3, &[0.0, 1.0, 0.5, 0.0, 1.0]) - 0.3).abs() < 1e-5);
        assert_eq!(levels_value(0.0, &q), 0.0);
        assert!((levels_value(1.0, &q) - 1.0).abs() < 1e-6);
        // output bounds outside 0..1 are limited first, not the result
        assert!((levels_value(0.5, &[0.0, 1.0, 0.5, 0.5, 1.225]) - 0.75).abs() < 1e-6);
        assert!((levels_value(0.5, &[0.0, 1.0, 0.5, -0.275, 0.4]) - 0.2).abs() < 1e-6);
        // inverted output range
        assert!((levels_value(0.0, &[0.0, 1.0, 0.5, 1.0, 0.0]) - 1.0).abs() < 1e-6);
        // low = high
        let step = [0.5, 0.5, 0.5, 0.0, 1.0];
        assert_eq!((levels_value(0.5, &step), levels_value(0.51, &step)), (0.0, 1.0));
    }

    #[test]
    fn a_float_node_keeps_output_bounds_outside_the_unit_range() {
        // 0.25 through output bounds -0.5..0.5: -0.25 in a node of floats, 0.125 in a 16-bit one (bounds limited to 0..0.5 first)
        let at = |ofmt: &str| {
            let text = format!("size 4\nnode 1 color r=0.25 g=0.25 b=0.25\nnode 2 levels_mid in=@1 out_low=-0.5 out_high=0.5 {ofmt}\noutput o 2\n");
            crate::graph::Graph::parse(&text).unwrap().evaluate(Some(4)).unwrap()["o"].px[0][0]
        };
        assert!((at("") + 0.25).abs() < 1e-6);
        assert!((at("ofmt=6") - 0.125).abs() < 1e-4);
    }

    #[test]
    fn hsl_neutral_keeps_the_colour() {
        let p = [0.8, 0.3, 0.1, 1.0];
        let o = hsl_shift(p, 0.5, 0.5, 0.5);
        for k in 0..3 {
            assert!((o[k] - p[k]).abs() < 1e-5, "{o:?}");
        }
        let grey = hsl_shift(p, 0.5, 0.0, 0.5);
        assert!((grey[0] - grey[1]).abs() < 1e-5 && (grey[1] - grey[2]).abs() < 1e-5);
    }

    #[test]
    fn hsl_adds_twice_the_distance_from_the_middle() {
        let p = [0.6, 0.2, 0.2, 1.0];
        let (h0, s0, l0) = rgb_to_hsl(p[0], p[1], p[2]);
        let q = hsl_shift(p, 0.6, 0.45, 0.4);
        let (h, s, l) = rgb_to_hsl(q[0], q[1], q[2]);
        assert!(((h - h0).rem_euclid(1.0) - 0.2).abs() < 1e-4, "{h} {h0}");
        assert!((s - (s0 - 0.1)).abs() < 1e-4, "{s} {s0}");
        assert!((l - (l0 - 0.2)).abs() < 1e-4, "{l} {l0}");
        assert_eq!(q[3], 1.0);
    }
}

#[cfg(test)]
mod blur_tests {
    use super::*;

    fn step() -> Tex {
        // a vertical edge in the middle of a row of 16
        let mut t = Tex::solid(16, 2, [0.0, 0.0, 0.0, 1.0]);
        for (i, p) in t.px.iter_mut().enumerate() {
            if i % 16 >= 8 {
                *p = [1.0, 1.0, 1.0, 1.0];
            }
        }
        t
    }

    #[test]
    fn a_box_covers_parts_of_the_outer_pixels() {
        // reach 1.4: the outer pixels count for 0.9 of a pixel each
        let t = box_axis(&step(), 1.4, true);
        assert!((t.px[7][0] - 0.9 / 2.8 - 0.0).abs() < 1e-5, "{}", t.px[7][0]);
        assert!((t.px[8][0] - (1.0 + 0.9) / 2.8).abs() < 1e-5, "{}", t.px[8][0]);
        assert_eq!(t.px[5][0], 0.0);
    }

    #[test]
    fn a_16_bit_box_reads_pairs_half_up_and_keeps_flat_images_flat() {
        let step = 1.0 / 65535.0;
        let row = |vals: &[f32]| Tex { w: vals.len(), h: 1, px: vals.iter().map(|v| [v * step; 4]).collect() };
        // width 5 at the first pixel: 10, then (1 + 2) and (3 + 4) rounded up to 2 and 4, each counting twice: (10 + 4 + 8) / 5 = 4.4, held at 4
        let mut v = [0.0f32; 16];
        v[0] = 10.0;
        v[1] = 1.0;
        v[2] = 2.0;
        v[15] = 3.0;
        v[14] = 4.0;
        let o = compiled_box_axis(&row(&v), 2.5, true, 6);
        assert!((o.px[0][0] / step - 4.0).abs() < 1e-3, "{}", o.px[0][0] / step);
        // the whole plan adds up to the width, at any width and position
        let flat = row(&[1000.0; 64]);
        for r in [1.25, 1.6, 2.0, 2.5, 3.2, 3.5, 4.5, 6.5, 10.5, 20.5] {
            let o = compiled_box_axis(&flat, r, true, 6);
            assert!(o.px.iter().all(|p| (p[0] / step - 1000.0).abs() < 0.05), "{r}: {}", o.px[1][0] / step);
        }
    }

    #[test]
    fn an_8_bit_blur_holds_its_first_pass_in_8_bits() {
        // one level on the right half: the pixel left of the step gets 0.32 of a level across, which rounds to nothing
        let level = 1.0 / 255.0;
        let t = Tex::from_fn(16, 16, |u, _| if u > 0.5 { [level, level, level, 1.0] } else { [0.0, 0.0, 0.0, 1.0] });
        let held = held_blur(&t, 1.4, 1.4, true, true, COMPILED_8_BIT);
        assert_eq!(held.px[8 * 16 + 7][0], 0.0);
        let float = box_axis(&box_columns(&t, 1.4, true), 1.4, true);
        assert!((float.px[8 * 16 + 7][0] - 0.9 / 2.8 * level).abs() < 1e-6);
    }

    #[test]
    fn the_defaults_are_ten_and_for_sharpen_one() {
        let run = |node: &str| {
            let g = format!(
                "size 256\nsubgraph s\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"s\"\nnode 2 {node} in=@1\noutput o 2\n"
            );
            crate::graph::Graph::parse(&g).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(256)).unwrap()["o"].clone()
        };
        let same = |a: &Tex, b: &Tex| a.px.iter().zip(&b.px).all(|(p, q)| (p[0] - q[0]).abs() < 1e-5);
        assert!(same(&run("box_blur"), &run("box_blur intensity=10")));
        assert!(same(&run("line_blur"), &run("line_blur intensity=10")));
        assert!(same(&run("laplacian_sharpen"), &run("laplacian_sharpen intensity=1")));
    }

    fn impulse(w: usize, h: usize, x: usize, y: usize) -> Tex {
        let mut t = Tex::solid(w, h, [0.0, 0.0, 0.0, 1.0]);
        t.px[y * w + x] = [1.0, 1.0, 1.0, 1.0];
        t
    }

    #[test]
    fn a_slanted_blur_shears_averages_and_shears_back() {
        // 1/8 turn is a tie, which goes to y: each row gets 1/4 1/2 1/4 across x, rows weigh by a box of half-width
        // 0.5 + (r - 0.5) cos, here r = 5
        let t = line_blur(&impulse(64, 64, 32, 32), 20.0, [1.0, 1.0], 0.125, true, true).expect("short enough");
        let r = 0.5 + 4.5 * std::f32::consts::FRAC_1_SQRT_2;
        let row = 1.0 / (2.0 * r);
        let at = |x: usize, y: usize| t.px[y * 64 + x][0];
        assert!((at(32, 32) - 0.5 * row).abs() < 1e-4, "{}", at(32, 32));
        assert!((at(31, 32) - 0.25 * row).abs() < 1e-4 && (at(33, 32) - 0.25 * row).abs() < 1e-4);
        assert!((at(34, 34) - 0.5 * row).abs() < 1e-4, "follows the line");
        assert_eq!(at(33, 31), 0.0);
        // the rows that the box reaches in part
        assert!((at(36, 36) - 0.5 * (r - 3.5) * row).abs() < 1e-4, "{}", at(36, 36));
        assert_eq!(at(37, 37), 0.0);
    }

    #[test]
    fn a_short_slanted_blur_reads_one_step_to_either_side() {
        // r = 1 at 1/16 turn: half-width 0.5 + 0.5 cos, the reads tan rows off
        let t = line_blur(&impulse(64, 64, 32, 32), 4.0, [1.0, 1.0], 0.0625, true, true).expect("short enough");
        let (s, c) = math::sin_cos(0.0625f32 * std::f32::consts::TAU);
        let r = 0.5 + 0.5 * c;
        let at = |x: usize, y: usize| t.px[y * 64 + x][0];
        assert!((at(32, 32) - 1.0 / (2.0 * r)).abs() < 1e-5);
        let side = 0.5 * c / (2.0 * r);
        assert!((at(33, 33) - side * (s / c * 256.0).round() / 256.0).abs() < 1e-5, "{}", at(33, 33));
        assert_eq!(at(32, 33), 0.0);
    }

    #[test]
    fn a_line_stays_straight_across_a_repeating_seam() {
        let t = line_blur(&impulse(64, 64, 0, 32), 40.0, [1.0, 1.0], 0.0625, true, true).expect("short enough");
        let mass: f32 = t.px.iter().map(|p| p[0]).sum();
        assert!((mass - 1.0).abs() < 1e-3);
        // the height of the line in a column: it runs through the impulse both ways, across the seam too
        let centre = |x: i64| {
            let col: Vec<f32> = (0..64).map(|y| t.at(x, y)[0]).collect();
            col.iter().enumerate().map(|(y, v)| y as f32 * v).sum::<f32>() / col.iter().sum::<f32>()
        };
        let tan = math::tan(0.0625f32 * std::f32::consts::TAU);
        assert!((centre(5) - (32.0 + 5.0 * tan)).abs() < 0.02, "{}", centre(5));
        assert!((centre(-5) - (32.0 - 5.0 * tan)).abs() < 0.02, "{}", centre(-5));
    }

    #[test]
    fn a_short_side_goes_along_the_image_not_along_the_angle() {
        // x major by the angle in a square, y major when the image is wider than it is tall by enough: |vx|/w < |vy|/h
        let t = line_blur(&impulse(256, 64, 128, 32), 3.3, [1.0, 1.0], 0.2, true, true).expect("short enough");
        let mass: f32 = t.px.iter().map(|p| p[0]).sum();
        assert!((mass - 1.0).abs() < 1e-3);
        assert_eq!(t.px[32 * 256 + 131][0], 0.0, "stays within the three rows");
    }

    #[test]
    fn a_transform_that_only_resizes_enlarges_by_doubling() {
        let t = doubled_to(&impulse(4, 4, 1, 1), 16, 16, true, true, 0);
        // twice 1 3 3 1 / 8 per axis, not one bilinear read: along a row through the middle
        let row: Vec<f32> = (0..8).map(|x| t.px[6 * 16 + x][0]).collect();
        let want = [0.0, 0.0625, 0.1875, 0.375, 0.625, 0.75, 0.75, 0.625].map(|v: f32| v * 0.75);
        assert!(row.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5), "{row:?}");
        assert!(is_axis_swap([1.0, 0.0, 0.0, -1.0]) && is_axis_swap([0.0, 1.0, 1.0, 0.0]));
        assert!(!is_axis_swap([1.0001, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn bilinear_weights_go_in_steps_of_a_256th() {
        let mut t = Tex::solid(4, 4, [0.0, 0.0, 0.0, 1.0]);
        t.px[1] = [1.0, 1.0, 1.0, 1.0];
        // a position 0.2 / 256 past the first pixel's centre rounds away, 0.6 / 256 past it rounds to 1 / 256
        let v = 0.5 / 4.0;
        assert_eq!(sample_stepped(&t, (0.5 + 0.2 / 256.0) / 4.0, v)[0], 0.0);
        assert!((sample_stepped(&t, (0.5 + 0.6 / 256.0) / 4.0, v)[0] - 1.0 / 256.0).abs() < 1e-6);
    }

    #[test]
    fn a_reach_up_to_half_a_pixel_changes_nothing() {
        assert_eq!(box_axis(&step(), 0.5, true).px[7][0], 0.0);
    }

    #[test]
    fn beyond_the_image_it_repeats_or_keeps_the_edge() {
        let t = box_axis(&step(), 2.0, true);
        assert!(t.px[0][0] > 0.0, "the bright end wraps around");
        let t = box_axis(&step(), 2.0, false);
        assert_eq!(t.px[0][0], 0.0);
        assert_eq!(t.px[15][0], 1.0);
    }

    #[test]
    fn the_reach_is_in_pixels_of_a_256_pixel_image() {
        let g = crate::graph::Graph::parse("size 32\nnode 1 color r=0.5 g=0.5 b=0.5\nnode 2 box_blur in=@1 intensity=8\noutput o 2\n")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!((g.evaluate(Some(32)).unwrap()["o"].px[100][0] - 0.5).abs() < 1e-5);
        // 16 of 256 at 16 px: reach 1
        assert!((box_axis(&step(), 16.0 * 16.0 / 256.0, true).px[7][0] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn aspect_scales_the_reach_per_axis() {
        let run = |extra: &str| {
            let g = format!(
                "size 32\nsubgraph s\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"s\"\nnode 2 box_blur in=@1 intensity=8 tile_x=0 {extra}\noutput o 2\n"
            );
            crate::graph::Graph::parse(&g).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(32)).unwrap()["o"].clone()
        };
        // zero leaves the image alone
        let plain = run("aspect_x=0");
        assert!((plain.px[0][0] - 0.5 / 32.0).abs() < 1e-4);
        assert!(run("").px[0][0] > plain.px[0][0] + 1e-3);
    }

    #[test]
    fn sharpen_is_the_difference_to_the_four_neighbours_limited() {
        let mut t = Tex::solid(8, 8, [0.5, 0.5, 0.5, 1.0]);
        t.px[3 * 8 + 3] = [0.6, 0.6, 0.6, 1.0];
        let o = laplacian_sharpen(&t, [0.25, 0.25], true, true);
        assert!((o.px[3 * 8 + 3][0] - (0.6 + 0.25 * 0.4)).abs() < 1e-5);
        assert!((o.px[3 * 8 + 2][0] - (0.5 - 0.25 * 0.1)).abs() < 1e-5);
        assert_eq!(laplacian_sharpen(&t, [100.0, 100.0], true, true).px[3 * 8 + 3][0], 1.0);
    }
}

#[cfg(test)]
mod gradient_tests {
    use crate::graph::Graph;

    fn at_half(extra: &str) -> f32 {
        let text = format!("size 8\nnode 1 color r=0.5 g=0.5 b=0.5\nnode 2 colorize in=@1 {extra}\noutput o 2\n");
        Graph::parse(&text).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(8)).unwrap()["o"].px[0][0]
    }

    #[test]
    fn a_table_of_256_entries_loses_a_peak_between_two_of_them() {
        // 0.5 falls halfway between entries 127 and 128, which lie on the feet of this narrow peak
        let keys = "p0=0.4985 r0=0 g0=0 b0=0 p1=0.5 r1=1 g1=1 b1=1 p2=0.5015 r2=0 g2=0 b2=0";
        assert!((at_half(keys) - 1.0).abs() < 1e-3);
        assert!(at_half(&format!("{keys} table=1")) < 0.01);
    }
}

#[cfg(test)]
mod blend_tests {
    use crate::tex::Tex;

    fn one(mode: u32, f: f32, b: f32, opacity: f32) -> f32 {
        let t = |v: f32| Tex::solid(1, 1, [v, v, v, 1.0]);
        crate::ops::blend(&t(f), &t(b), None, mode, opacity).px[0][0]
    }

    #[test]
    fn add_subtract_and_add_sub_scale_the_foreground() {
        assert!((one(32, 0.8, 0.5, 0.5) - 0.9).abs() < 1e-6);
        assert!((one(32, 0.8, 0.5, 1.0) - 1.0).abs() < 1e-6);
        assert!((one(33, 0.8, 0.5, 0.5) - 0.1).abs() < 1e-6);
        assert!((one(34, 0.25, 0.5, 0.5) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn divide_mixes_before_limiting() {
        // mixing 0.8 with 4 halfway is 2.4, so 1, and not the 0.9 of a mix with the limited 1
        assert_eq!(one(35, 0.2, 0.8, 0.5), 1.0);
        assert!((one(35, 0.8, 0.2, 0.5) - 0.225).abs() < 1e-6);
    }

    #[test]
    fn soft_light_uses_the_square_root_at_all_levels() {
        let v = one(36, 0.75, 0.04, 1.0);
        assert!((v - (0.04 + 0.5 * (0.2 - 0.04))).abs() < 1e-5, "{v}");
        assert!((one(36, 0.25, 0.5, 1.0) - (0.5 - 0.5 * 0.25)).abs() < 1e-6);
    }
}

#[cfg(test)]
mod transform_tests {
    use crate::graph::Graph;

    /// A pixel of a transform of the image `f(u)` whose function is written in `body`.
    fn run(node: &str) -> crate::tex::Tex {
        let text = format!(
            "size 64\nsubgraph ramp\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"ramp\"\nnode 2 transform_matrix in=@1 {node}\noutput o 2\n"
        );
        Graph::parse(&text).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(64)).unwrap()["o"].clone()
    }

    #[test]
    fn the_matrix_columns_give_the_input_position_and_the_offset_adds_to_it() {
        // the input is a ramp in x: reading at m00 x + m10 y + offset_x from the centre
        let t = run("m00=0.5 m10=0.25 m01=9 m11=9 offset_x=0.125");
        let (x, y) = ((20.5f32 / 64.0) - 0.5, (10.5f32 / 64.0) - 0.5);
        assert!((t.px[10 * 64 + 20][0] - (0.5 * x + 0.25 * y + 0.5 + 0.125)).abs() < 1e-5);
    }

    #[test]
    fn an_axis_that_does_not_repeat_is_black_outside() {
        let t = run("m00=2 m11=2 tile_x=0 tile_y=1");
        assert_eq!(t.px[5][0], 0.0);
        let t = run("m00=2 m11=2 tile_x=1 tile_y=0");
        assert_ne!(t.px[32 * 64 + 5][0], 0.0);
        assert_eq!(t.px[5][0], 0.0, "and on the other axis");
    }

    #[test]
    fn outside_the_image_shows_the_background() {
        let t = run("m00=2 m11=2 tile_x=0 tile_y=0 bg_r=0.5 bg_g=0.25 bg_b=0.75 bg_a=1");
        assert_eq!(t.px[0], [0.5, 0.25, 0.75, 1.0]);
    }

    #[test]
    fn an_edge_that_does_not_repeat_fades_over_one_pixel_and_keeps_its_value_otherwise() {
        let t = crate::tex::Tex::solid(16, 16, [0.5, 0.5, 0.5, 1.0]);
        let bg = [0.0; 4];
        // at the same size the edge pixel's centre is half way out only at the very edge of the image
        assert_eq!(super::sample_bilinear(&t, 0.5 / 16.0, 0.5, false, true, bg, [16, 16])[0], 0.5);
        assert_eq!(super::sample_bilinear(&t, 0.0, 0.5, false, true, bg, [16, 16])[0], 0.25);
        assert_eq!(super::sample_bilinear(&t, -1.0 / 16.0, 0.5, false, true, bg, [16, 16])[0], 0.0);
        // enlarged to 512, the fade is a pixel of the result, so the first result pixel is whole
        assert_eq!(super::sample_bilinear(&t, 0.5 / 512.0, 0.5, false, true, bg, [512, 512])[0], 0.5);
    }

    #[test]
    fn a_level_beyond_the_image_stops_at_one_pixel() {
        let t = crate::tex::Tex::solid(4, 2, [0.5, 0.5, 0.5, 1.0]);
        let m = crate::patterns::mip_of(&t, 5).unwrap();
        assert_eq!((m.w, m.h), (1, 1));
        assert!(crate::patterns::mip_of(&t, 0).is_none());
    }

    #[test]
    fn nearest_reads_whole_pixels() {
        let t = run("m00=0.3 m11=0.3 filter=0");
        let x = 0.3 * ((20.5f32 / 64.0) - 0.5) + 0.5;
        assert!((t.px[10 * 64 + 20][0] - ((x * 64.0).floor() + 0.5) / 64.0).abs() < 1e-5);
    }

    #[test]
    fn the_level_follows_the_longer_axis_and_can_be_fixed() {
        assert_eq!(super::transform_mip(31, [1.3, 0.0, 0.0, 1.0]), 0);
        assert_eq!(super::transform_mip(31, [3.1, 0.0, 0.0, 0.7]), 2);
        assert_eq!(super::transform_mip(31, [0.0, 2.5, 0.1, 0.0]), 1);
        assert_eq!(super::transform_mip(3, [1.0, 0.0, 0.0, 1.0]), 3);
    }

    #[test]
    fn a_level_is_the_two_by_two_mean() {
        let t = run("mip=1");
        // pixels 20 and 21 of the ramp fall into one pixel of level 1, and bilinear reads between two of those
        let a = (20.5f32 + 21.5) / 2.0 / 64.0;
        let b = (22.5f32 + 23.5) / 2.0 / 64.0;
        // at output pixel 21 the position is 21.5/32 of level 1 = halfway between level pixels 10 and 11 (shifted by 0.25)
        let at = run("m00=1 m11=1 filter=0 mip=1");
        assert!((at.px[10 * 64 + 21][0] - a).abs() < 1e-5, "{} {a}", at.px[10 * 64 + 21][0]);
        assert!((t.px[10 * 64 + 21][0] - (a + (b - a) * 0.25)).abs() < 1e-5, "{}", t.px[10 * 64 + 21][0]);
    }
}

#[cfg(test)]
mod dir_warp_tests {
    use crate::graph::Graph;

    fn render(warp: &str) -> crate::tex::Tex {
        // a vertical edge at x = 0.5, moved by a uniform map at half brightness
        let text =
            format!("size 64\nnode 1 gradient mode=0 angle=0\nnode 2 color r=0.5 g=0.5 b=0.5\nnode 3 dir_warp in=@1 map=@2 {warp}\noutput o 3\n");
        Graph::parse(&text).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(64)).unwrap()["o"].clone()
    }

    #[test]
    fn intensity_is_in_256ths_of_the_image_per_unit_of_map() {
        let (plain, moved) = (render("intensity=0"), render("intensity=32"));
        // 32/256 * 0.5 = 1/16 of the image, which is 4 pixels at 64
        assert!((moved.px[10 * 64 + 20][0] - plain.px[10 * 64 + 24][0]).abs() < 1e-5);
    }

    #[test]
    fn a_map_offset_takes_away_from_the_brightness() {
        let (plain, moved) = (render("intensity=0"), render("intensity=64 map_offset=0.25"));
        // 64/256 * (0.5 - 0.25) = 1/16 of the image
        assert!((moved.px[10 * 64 + 20][0] - plain.px[10 * 64 + 24][0]).abs() < 1e-5);
    }

    #[test]
    fn the_aspect_scales_the_move_along_each_axis() {
        let plain = render("intensity=0");
        // aspect 2 across: 2 * 32/256 * 0.5 = 1/8 of the image, 8 pixels at 64; the aspect down has nothing to move here
        let moved = render("intensity=32 aspect_x=2 aspect_y=0.25");
        assert!((moved.px[10 * 64 + 20][0] - plain.px[10 * 64 + 28][0]).abs() < 1e-5);
        let down = render("intensity=32 angle=0.25 aspect_x=0.25 aspect_y=0.5");
        assert!((down.px[20 * 64 + 10][0] - plain.px[22 * 64 + 10][0]).abs() < 1e-5);
    }

    #[test]
    fn a_quarter_turn_moves_along_the_other_axis() {
        let (plain, moved) = (render("intensity=0"), render("intensity=32 angle=0.25"));
        assert!((moved.px[20 * 64 + 10][0] - plain.px[24 * 64 + 10][0]).abs() < 1e-5);
    }
}

#[cfg(test)]
mod grad_warp_tests {
    use crate::graph::Graph;

    #[test]
    fn moves_by_the_backward_slope_of_the_map() {
        // a uv ramp as the map has slope 1 per uv, so intensity 16 moves by 1/16 of the image, 4 pixels at 64
        let g = "size 64\nsubgraph ramp\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"ramp\"\nnode 2 pixel_processor fn=\"ramp\"\nnode 3 grad_warp in=@1 map=@2 intensity=16\noutput o 3\n";
        let g = Graph::parse(g).unwrap_or_else(|e| panic!("{e}"));
        let t = &g.evaluate(Some(64)).unwrap()["o"];
        let input = (20.0 + 0.5) / 64.0;
        assert!((t.px[10 * 64 + 20][0] - (input + 4.0 / 64.0)).abs() < 2e-3, "{}", t.px[10 * 64 + 20][0]);
    }

    /// The ramp's jump from 1 back to 0 at the left edge: wrapped it is a huge slope that throws the first column's read
    /// far away; an axis that doesn't repeat has no slope at its edge, so the first column keeps its own value.
    fn first_column(tile: &str) -> (f32, f32) {
        let g = format!(
            "size 64\nsubgraph ramp\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"ramp\"\nnode 2 pixel_processor fn=\"ramp\"\nnode 3 grad_warp in=@1 map=@2 intensity=16 {tile}\noutput o 3\n"
        );
        let g = Graph::parse(&g).unwrap_or_else(|e| panic!("{e}"));
        let t = &g.evaluate(Some(64)).unwrap()["o"];
        (t.px[10 * 64][0], t.px[10 * 64 + 20][0])
    }

    #[test]
    fn a_sampler_with_nothing_connected_reads_zero() {
        let g = "size 8\nsubgraph f\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_sample_gray a=@2 slot=\"in2\" filter=0\nnode 4 f_float v=0.25\nnode 5 f_add a=@3 b=@4\nnode 1 f_out in=@5\nend\nnode 1 color r=1 g=1 b=1\nnode 2 pixel_processor fn=\"f\" in=@1\noutput o 2\n";
        let g = Graph::parse(g).unwrap_or_else(|e| panic!("{e}"));
        let t = &g.evaluate(Some(8)).unwrap()["o"];
        assert!((t.px[9][0] - 0.25).abs() < 1e-3, "{:?}", t.px[9]);
    }

    #[test]
    fn an_axis_that_does_not_repeat_has_no_slope_across_its_edge() {
        let (edge, inside) = first_column("tile_x=0 tile_y=0");
        assert!((edge - 0.5 / 64.0).abs() < 2e-3, "{edge}");
        // inside, the same move as a repeating map
        assert!((inside - (20.5 + 4.0) / 64.0).abs() < 2e-3, "{inside}");
        let (wrapped, _) = first_column("");
        assert!((wrapped - edge).abs() > 0.05, "{wrapped}");
    }
}

#[cfg(test)]
mod compiled_size_tests {
    use crate::graph::Graph;

    fn run(graph: &str) -> crate::tex::Tex {
        Graph::parse(graph).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(8)).unwrap()["o"].clone()
    }

    // a ramp in x at 4 pixels, then a levels at 8 that changes nothing
    const RAMP: &str = "subgraph ramp\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"ramp\" fn.osize_log2_x=\"two\" fn.osize_log2_y=\"two\"\nsubgraph two\nmeta kind \"function\"\nnode 2 f_float v=2\nnode 1 f_out in=@2\nend\n";

    #[test]
    fn enlarging_keeps_the_edge_in_a_compiled_graph_and_wraps_otherwise() {
        let body = format!("{RAMP}node 2 levels_mid in=@1 osize_mode=1\noutput o 2\n");
        let plain = run(&format!("size 8\n{body}"));
        let compiled = run(&format!("size 8\nmeta compiled \"1\"\n{body}"));
        // the first pixel of the 8 is a quarter of a source pixel before the first centre: it stays at the first value
        assert!((compiled.px[0][0] - 0.125).abs() < 1e-5, "{}", compiled.px[0][0]);
        assert!(plain.px[0][0] > 0.125, "{}", plain.px[0][0]);
    }

    #[test]
    fn a_transform_in_a_compiled_graph_reads_the_input_at_its_own_size() {
        // nearest on the 4 pixels of the ramp, at 8 pixels: pairs of equal values
        let t = run(&format!("size 8\nmeta compiled \"1\"\n{RAMP}node 2 transform_matrix in=@1 filter=0 mip=0 osize_mode=1\noutput o 2\n"));
        assert_eq!((t.w, t.h), (8, 8));
        assert_eq!(t.px[0][0], t.px[1][0]);
        assert_eq!(t.px[2][0], t.px[3][0]);
        assert!((t.px[2][0] - 0.375).abs() < 1e-5);
    }

    #[test]
    fn nearest_filtering_with_an_automatic_mip_repeats_pixels_when_it_enlarges() {
        let at = |rest: &str| run(&format!("size 8\nmeta compiled \"1\"\n{RAMP}node 2 transform_matrix in=@1 {rest} osize_mode=1\noutput o 2\n"));
        let t = at("filter=0 mip=31");
        let row: Vec<f32> = (0..8).map(|x| t.px[x][0]).collect();
        assert_eq!(row, [0.125, 0.125, 0.375, 0.375, 0.625, 0.625, 0.875, 0.875]);
        // with the flip it reads the same pixels the other way round
        let t = at("filter=0 mip=31 m00=-1");
        let row: Vec<f32> = (0..8).map(|x| t.px[x][0]).collect();
        assert_eq!(row, [0.875, 0.875, 0.625, 0.625, 0.375, 0.375, 0.125, 0.125]);
        // bilinear keeps doubling: no two neighbours alike in the middle
        let t = at("filter=1 mip=31");
        assert_ne!(t.px[2][0], t.px[3][0]);
    }

    #[test]
    fn a_size_changing_unequally_reduces_each_axis_on_its_own() {
        use super::reduced_per_axis;
        let mut t = crate::tex::Tex::solid(8, 8, [0.0, 0.0, 0.0, 1.0]);
        for y in 0..8 {
            for x in 0..8 {
                t.px[y * 8 + x] = [x as f32 / 8.0, y as f32 / 8.0, 0.0, 1.0];
            }
        }
        let o = reduced_per_axis(&t, 8, 2);
        assert_eq!((o.w, o.h), (8, 2));
        // x untouched, y the mean of four rows
        assert!((o.px[3][0] - 3.0 / 8.0).abs() < 1e-4);
        assert!((o.px[3][1] - 1.5 / 8.0).abs() < 1e-4);
        assert!((o.px[8 + 3][1] - 5.5 / 8.0).abs() < 1e-4);
        // an axis smaller than the size stays, for doubling to take on
        let o = reduced_per_axis(&t, 16, 4);
        assert_eq!((o.w, o.h), (8, 4));
    }

    #[test]
    fn a_small_pixel_processor_in_a_compiled_graph_reads_the_graphs_size() {
        // a node of 4 x 4 in a graph of 8: f_size is the graph's, f_own_size the node's
        let at = |kind: &str, meta: &str| {
            let text = format!(
                "size 8\n{meta}subgraph f\nmeta kind \"function\"\nnode 2 {kind}\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 4 f_float v=8\nnode 5 f_div_large a=@3 b=@4\nnode 1 f_out in=@5\nend\nsubgraph two\nmeta kind \"function\"\nnode 2 f_float v=2\nnode 1 f_out in=@2\nend\nnode 1 pixel_processor fn=\"f\" fn.osize_log2_x=\"two\" fn.osize_log2_y=\"two\"\noutput o 1\n"
            );
            let t = run(&text);
            assert_eq!((t.w, t.h), (4, 4));
            t.px[0][0]
        };
        assert_eq!(at("f_size", "meta compiled \"1\"\n"), 1.0);
        assert_eq!(at("f_own_size", "meta compiled \"1\"\n"), 0.5);
        // a graph of our own keeps the node's size
        assert_eq!(at("f_size", ""), 0.5);
    }

    #[test]
    fn sharpen_in_a_compiled_graph_reads_neighbours_one_input_pixel_away() {
        // on a ramp the differences cancel, in any pixel of a straight stretch
        let t = run(&format!(
            "size 8\nmeta compiled \"1\"\n{RAMP}node 2 laplacian_sharpen in=@1 tile_x=0 tile_y=0 intensity=0.5 osize_mode=1\noutput o 2\n"
        ));
        assert_eq!((t.w, t.h), (8, 8));
        assert!((t.px[8 * 2 + 3][0] - (3.5 / 8.0)).abs() < 1e-4, "{}", t.px[8 * 2 + 3][0]);
    }

    #[test]
    fn a_pixel_processor_in_a_compiled_graph_samples_the_input_at_its_own_size() {
        let f = "subgraph s\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_sample_gray a=@2 filter=0\nnode 1 f_out in=@3\nend\n";
        let t = run(&format!("size 8\nmeta compiled \"1\"\n{RAMP}{f}node 2 pixel_processor fn=\"s\" in=@1 osize_mode=1\noutput o 2\n"));
        assert_eq!(t.px[0][0], t.px[1][0]);
        assert!((t.px[2][0] - 0.375).abs() < 2e-3, "{}", t.px[2][0]);
    }

    #[test]
    fn a_level_the_transform_chooses_fades_at_an_edge_over_its_own_pixel() {
        // an 8 x 8 white squeezed by 3 at level 3 (1 x 1), not repeating vertically, read bilinearly over black
        let g = Graph::parse("size 8\nmeta compiled \"1\"\nnode 1 color r=1 g=1 b=1 osize_log2_x=3 osize_log2_y=3\nnode 2 transform_matrix tile_x=1 tile_y=0 filter=1 mip=3 in=@1 m00=1 m01=0 m10=0 m11=3 osize_log2_x=3 osize_log2_y=3\noutput o 2\n").unwrap();
        let t = &g.evaluate(Some(8)).unwrap()["o"];
        let col: Vec<f32> = (0..8).map(|y| t.px[y * 8][0]).collect();
        let want = [0.0, 0.0625, 0.4375, 0.8125, 0.8125, 0.4375, 0.0625, 0.0];
        assert!(col.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-3), "{col:?}");
    }

    #[test]
    fn an_enlarging_transform_fades_over_a_pixel_of_the_result_at_any_level() {
        // baked_lighting_material: an 8 x 8 enlarged at level 1, not repeating vertically; the level's pixel would fade a quarter of the image
        let g = Graph::parse("size 64\nmeta compiled \"1\"\nnode 1 color r=1 g=1 b=1 osize_log2_x=3 osize_log2_y=3\nnode 2 transform_matrix tile_x=1 tile_y=0 filter=1 mip=1 in=@1 osize_log2_x=6 osize_log2_y=6\noutput o 2\n").unwrap();
        let t = &g.evaluate(Some(64)).unwrap()["o"];
        assert!(t.px[4 * 64][0] > 0.99, "{}", t.px[4 * 64][0]);
    }
}

#[cfg(test)]
mod compiled_function_tests {
    use crate::graph::Graph;

    fn value(nodes: &str) -> f32 {
        let g = format!("size 8\nsubgraph f\nmeta kind \"function\"\n{nodes}end\nnode 1 color fn.r=\"f\"\noutput o 1\n");
        Graph::parse(&g).unwrap_or_else(|e| panic!("{e}\n{g}")).evaluate(Some(8)).unwrap()["o"].px[0][0]
    }

    #[test]
    fn dividing_by_zero_and_the_log_of_zero_saturate() {
        let huge = value("node 2 f_float v=0.5\nnode 3 f_float v=0\nnode 4 f_div_large a=@2 b=@3\nnode 1 f_out in=@4\n");
        assert!(huge > 1e20, "{huge}");
        let low = value("node 2 f_float v=0\nnode 3 f_log_floor a=@2\nnode 1 f_out in=@3\n");
        assert!(low < -1e20, "{low}");
        // and nothing but zero over zero is zero
        assert_eq!(value("node 2 f_float v=0\nnode 3 f_div_large a=@2 b=@2\nnode 1 f_out in=@3\n"), 0.0);
    }

    #[test]
    fn float_mod_works_in_single_precision() {
        // 1.0 mod 0.2 is 0 in single precision, nearly 0.2 in double
        let v = value("node 2 f_float v=1\nnode 3 f_float v=0.2\nnode 4 f_mod a=@2 b=@3\nnode 1 f_out in=@4\n");
        assert!(v.abs() < 1e-5, "{v}");
    }

    fn sampled(filter: &str, at: [f32; 2]) -> f32 {
        // the ramp in x on 4 pixels: 0.125 0.375 0.625 0.875
        let text = format!(
            "size 4\nsubgraph ramp\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nsubgraph s\nmeta kind \"function\"\nnode 2 f_vec2 x={} y={}\nnode 3 f_sample_gray a=@2 {filter}\nnode 1 f_out in=@3\nend\nnode 1 pixel_processor fn=\"ramp\"\nnode 2 pixel_processor fn=\"s\" in=@1\noutput o 2\n",
            at[0], at[1]
        );
        Graph::parse(&text).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(4)).unwrap()["o"].px[0][0]
    }

    #[test]
    fn the_filter_picks_nearest_or_bilinear_and_repeat_or_edge() {
        assert!((sampled("filter=0", [0.3, 0.5]) - 0.375).abs() < 1e-3);
        assert!((sampled("filter=1", [0.25, 0.5]) - 0.25).abs() < 1e-3);
        // past the right end: repeats to the first pixel, or stays on the last
        assert!((sampled("filter=0", [1.1, 0.5]) - 0.125).abs() < 1e-3);
        assert!((sampled("filter=2", [1.1, 0.5]) - 0.875).abs() < 1e-3);
        assert!((sampled("filter=3", [1.1, 0.5]) - 0.875).abs() < 1e-3);
        // without a filter: bilinear and repeating, as always
        assert!((sampled("", [0.25, 0.5]) - 0.25).abs() < 1e-3);
    }
}

#[cfg(test)]
mod transform_mip_tests {
    use super::transform_mip;

    #[test]
    fn codes_30_and_31_take_the_level_from_the_scale() {
        let half = [4.0, 0.0, 0.0, 4.0];
        assert_eq!((transform_mip(30, half), transform_mip(31, half), transform_mip(3, half)), (2, 2, 3));
    }
}

#[cfg(test)]
mod own_size_tests {
    #[test]
    fn a_parameter_reads_the_size_of_its_node() {
        let text = "size 64\nsubgraph f\nmeta kind \"function\"\nnode 2 f_own_size\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 4 f_float v=64\nnode 5 f_div_large a=@3 b=@4\nnode 1 f_out in=@5\nend\nnode 1 color fn.r=\"f\" g=0 b=0 osize_log2_x=4 osize_log2_y=4\noutput o 1\n";
        let t = &crate::graph::Graph::parse(text).unwrap().evaluate(Some(64)).unwrap()["o"];
        assert_eq!(t.px[0][0], 0.25);
    }
}
