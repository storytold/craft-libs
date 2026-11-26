// SPDX-License-Identifier: MIT OR Apache-2.0
//! Node graph: text format, validation, subgraphs, packages and evaluation.
use crate::accelerator::{
    ColorInput, GreyInput, GreyTransformInput, HalfTransform, PixelImageInput, ResidentColor16, ResidentColorOutput, ResidentColorPass,
    ResidentGrey16, ResidentGreyFloat, ResidentGreyPass,
};
pub use crate::context::Logging;
use crate::context::{EvalContext, LiveBytes};
use crate::expr;
use crate::grunge;
use crate::math;
use crate::noise;
use crate::ops;
use crate::png;
use crate::scatter;
use crate::tex::{Grey, Tex, Val, lum};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::Arc as Rc;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug)]
pub enum Error {
    Parse(usize, String),
    UnknownNode(u32, String),
    UnknownSlot(u32, String),
    MissingInput(u32, String),
    MissingNode(u32, u32),
    Cycle(u32),
    BadParam(u32, String),
    GpuReadback(u32, String),
    NoOutputs,
    BadSize(usize),
    Io(String),
    /// the editor started a newer evaluation; this one stopped early
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(l, m) => write!(f, "line {l}: {m}"),
            Error::UnknownNode(id, k) => write!(f, "node {id}: unknown node type '{k}'"),
            Error::UnknownSlot(id, s) => write!(f, "node {id}: unknown input slot '{s}'"),
            Error::MissingInput(id, s) => write!(f, "node {id}: required input '{s}' is not connected"),
            Error::MissingNode(id, r) => write!(f, "node {id}: refers to missing node {r}"),
            Error::Cycle(id) => write!(f, "cycle detected at node {id}"),
            Error::BadParam(id, m) => write!(f, "node {id}: {m}"),
            Error::GpuReadback(id, m) => write!(f, "node {id}: GPU readback failed: {m}"),
            Error::NoOutputs => write!(f, "graph has no outputs"),
            Error::BadSize(s) => write!(f, "invalid resolution {s} (use 1..=8192)"),
            Error::Io(m) => write!(f, "io error: {m}"),
            Error::Cancelled => write!(f, "evaluation cancelled"),
        }
    }
}
impl std::error::Error for Error {}

pub struct Spec {
    pub kind: &'static str,
    pub req: &'static [&'static str],
    pub opt: &'static [&'static str],
    pub doc: &'static str,
}

/// Slot names of a node's image inputs by position, on the nodes that take many (decompiled pixel processors and fx-maps).
pub const IMAGE_SLOTS: &[&str] = &[
    "in", "in2", "in3", "in4", "in5", "in6", "in7", "in8", "in9", "in10", "in11", "in12", "in13", "in14", "in15", "in16", "in17", "in18", "in19",
    "in20", "in21", "in22", "in23", "in24", "in25", "in26", "in27", "in28", "in29", "in30", "in31", "in32",
];

macro_rules! s {
    ($k:expr_2021, [$($r:expr_2021),*], [$($o:expr_2021),*], $d:expr_2021) => {
        Spec { kind: $k, req: &[$($r),*], opt: &[$($o),*], doc: $d }
    };
}

pub const SPECS: &[Spec] = &[
    s!("color", [], [], "constant colour: r g b a"),
    s!("white_noise", [], [], "per-pixel random: seed distribution(0 uniform,1 gaussian,2 triangle)"),
    s!(
        "gaussian_noise",
        [],
        [],
        "gaussian-distributed per-pixel noise: seed scale contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y"
    ),
    s!("perlin", [], [], "perlin noise: scale scale_x scale_y seed disorder disorder_speed tile_x tile_y"),
    s!(
        "fbm",
        [],
        [],
        "fractal noise/clouds: scale octaves gain seed mode(0 fbm,1 ridged,2 turbulence) disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y"
    ),
    s!(
        "voronoi",
        [],
        [],
        "cellular: scale jitter seed mode(0 f1,1 cell id,2 edge,3 f2,4 f2-f1,5 f1*f2,6 f1/f2) aspect offset_x offset_y distortion distortion_scale distance_mode(0 euclid,1 manhattan,2 chebyshev,3 minkowski) minkowski edge_thickness rounded distance_scale color_seed_mode color_seed invert"
    ),
    s!(
        "voronoi_fractal",
        [],
        [],
        "layered voronoi: scale jitter seed mode aspect offset_x offset_y distortion distortion_scale distance_mode minkowski edge_thickness rounded distance_scale min_level max_level roughness lacunarity opacity blend(0 add,1 max,2 min) color_seed_mode color_seed invert"
    ),
    s!("gradient", [], [], "gradient: mode(0 linear,1 radial) angle"),
    s!("circle", [], [], "radius softness"),
    s!("rect", [], [], "width height softness"),
    s!("polygon", [], [], "sides radius softness rotation"),
    s!("checker", [], [], "tiles"),
    s!("bricks", [], [], "rows cols mortar bevel offset lum_var seed"),
    s!(
        "fx_map",
        [],
        ["in", "in2", "in3", "in4"],
        "run a pattern program: program=\"name\" depth seed blend (0 max 1 add 2 over 3 min 4 multiply) color bg bg_r bg_g bg_b"
    ),
    s!("curve", ["in"], [], "tone curves: points (master), points_r points_g points_b points_a as \"x:y,x:y\" lists"),
    s!("svg", [], [], "vector image: file=\"art.svg\" fit (0 contain 1 stretch 2 cover) scale x y"),
    s!(
        "text",
        [],
        [],
        "text: text=\"...\" font=\"face.ttf\" (optional) size x y (y_up=1: y counts up) align (0 left 1 centre 2 right) spacing line_spacing rotation value"
    ),
    s!(
        "channel_shuffle",
        ["in"],
        ["in2"],
        "rebuild channels: src_r src_g src_b src_a (0-3 channels of in, 4-7 channels of in2, 8 zero, 9 one), inv_r inv_g inv_b inv_a"
    ),
    s!(
        "input_value",
        [],
        [],
        "graph parameter read by functions: name, type (0 float, 1 int, 2 bool, 3 float2, 4 float3, 5 float4, 6 int2, 7 int3, 8 int4, 9 string), v / x y z w, min max, label"
    ),
    Spec {
        kind: "pixel_processor",
        req: &[],
        opt: IMAGE_SLOTS,
        doc: "per-pixel expr / expr_g / expr_b / expr_a (string params), or fn=\"function\" to run a function graph per pixel",
    },
    s!(
        "value_processor",
        [],
        ["in", "in2", "in3", "in4"],
        "run fn=\"function\" once; the result (float, float3 or float4) fills the image. The function can read image statistics"
    ),
    s!("sdf_shape", [], [], "2D signed distance shape: shape(0 circle,1 box,2 polygon,3 ring) size aspect sides rotation round x y range"),
    s!("sdf_op", ["a", "b"], [], "combine SDFs: op(0 union,1 subtract,2 intersect) smooth range"),
    s!("sdf_mask", ["in"], [], "SDF -> mask: offset softness"),
    s!("sdf_form", [], [], "procedural 3D SDF form as height: shape(0 sphere,1 box,2 torus,3 cylinder) size repeat round scatter seed height"),
    s!("bitmap", [], [], "flip_x flip_y gray; imported PNG bitmap: file=\"path.png\""),
    s!("input", [], [], "subgraph input: idx"),
    s!("subgraph", [], ["in0", "in1", "in2", "in3"], "instance of a reusable subgraph: graph=\"name\""),
    s!("shape", [], [], "procedural shape: square, disc, paraboloid, gaussian, diamond, pyramid, half sphere, cone"),
    s!(
        "tile_sampler",
        [],
        [
            "pattern",
            "pattern2",
            "pattern3",
            "pattern4",
            "pattern5",
            "pattern6",
            "mask",
            "scale_map",
            "rotation_map",
            "displacement_map",
            "color_map",
            "distribution_map",
            "background"
        ],
        "grid of pattern instances: x_amount y_amount size aspect size_mode scale scale_random size_random rotation rotation_random pattern_rotation symmetry_random position_random offset offset_mode offset_random global_offset_x global_offset_y color color_random lum_by_number lum_by_scale mask_threshold mask_invert mask_random mask_checker invert_mask blending opacity background reverse_order pattern_type pattern_specific pattern_distribution"
    ),
    s!(
        "tile_generator",
        [],
        [
            "pattern",
            "pattern2",
            "pattern3",
            "pattern4",
            "pattern5",
            "pattern6",
            "mask",
            "scale_map",
            "rotation_map",
            "displacement_map",
            "color_map",
            "distribution_map",
            "background"
        ],
        "brick-style tiles: x_amount y_amount size aspect size_mode scale scale_random size_random rotation rotation_random pattern_rotation symmetry_random position_random offset offset_mode offset_random global_offset_x global_offset_y color color_random lum_by_number lum_by_scale mask_checker mask_horizontal mask_vertical mask_random invert_mask blending opacity background reverse_order pattern_type pattern_specific pattern_distribution"
    ),
    s!(
        "splatter",
        [],
        ["pattern", "pattern2", "pattern3", "pattern4", "pattern5", "pattern6"],
        "random scatter of a pattern: count grid_number scale scale_var aspect rotation rot_var pattern_rotation lum_var disorder pan_x pan_y symmetry_random gain output_min output_max background blending opacity pattern_type pattern_specific pattern_distribution"
    ),
    s!(
        "splatter_circular",
        [],
        ["pattern", "pattern2", "pattern3", "pattern4", "pattern5", "pattern6"],
        "instances on one or more circles: number amount_random rings radius radius_random ring_radius_multiplier angle_random spiral_factor spread size size_random scale_by_number scale_by_ring rotation rotation_random center_orientation ring_rotation ring_rotation_random position_random color luminance_random lum_by_number lum_by_ring mask_random background blending opacity pattern_type pattern_specific pattern_distribution"
    ),
    s!("clouds", [], [], "soft cloud noise: scale seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y"),
    s!(
        "bnw_spots",
        [],
        [],
        "black and white spots: scale seed contrast invert variant disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y"
    ),
    s!("crystal", [], [], "faceted crystal cells: scale disorder seed contrast invert variant"),
    s!(
        "directional_noise",
        [],
        [],
        "streak noise, seamless at any angle: scale stretch angle seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y"
    ),
    s!(
        "grunge",
        [],
        [],
        "grunge masks: variant(0 concrete,1 scratches,2 spots,3 smudges) balance large_variation base_noisiness dirt_specks_opacity invert_dirt scratches_opacity sharpen quantity scratch_width scratch_length details coverage coverage_contrast disorder disorder_speed seed contrast invert"
    ),
    s!("plasma", [], [], "smooth plasma waves"),
    s!("stripes", [], [], "stripes: count angle softness width tile(1 = snap to seamless angle, 0 = exact)"),
    s!("weave", [], [], "woven threads: count width"),
    s!("star", [], [], "star shape: points inner size softness rotation"),
    s!("gradient_axial", [], [], "symmetric gradient: angle"),
    s!("gradient_circular", [], [], "angular gradient: offset"),
    s!("switch", [], ["a", "b", "c", "d"], "pass one of up to four inputs: index"),
    s!("flood_fill", ["in"], [], "label connected regions of a mask"),
    s!("flood_fill_random", ["in"], [], "random grayscale per flood-fill region"),
    s!("flood_fill_color", ["in"], [], "random color per flood-fill region"),
    s!("flood_fill_to_index", ["in"], [], "region number of each flood-fill region as a gray level"),
    s!("flood_fill_to_position", ["in"], [], "centre of each flood-fill region (outputs: position, x, y)"),
    s!("flood_fill_to_bbox_size", ["in"], [], "bounding-box size of each flood-fill region (outputs: size, width, height)"),
    s!("flood_fill_gradient", ["in"], [], "gradient across each flood-fill region"),
    s!("vector_warp", ["in", "vector"], [], "displace by a vector map: intensity directx"),
    s!("slope_blur", ["in", "slope"], [], "blur along a slope map: samples intensity mode"),
    s!("blur_hq", ["in"], [], "high quality blur: intensity quality"),
    s!("histogram_range", ["in"], [], "select a range of values: position range contrast invert"),
    s!("bevel", ["in"], [], "bevel a shape: distance smoothing threshold use_as_normal normal_intensity corner(0 round,1 angular) directx"),
    s!("distance", ["in"], [], "distance field of a mask: distance mode"),
    s!(
        "color_match",
        ["in"],
        ["reference", "src_color", "target_color"],
        "recolour: method(0 statistics transfer from reference,1 colour swap) strength source_mode(0 average,1 colour,2 input) src_r src_g src_b target_mode(0 colour,1 input) target_r target_g target_b custom_variation variation_hue variation_chroma variation_luma use_mask mask_hue_range mask_chroma_range mask_luma_range mask_smoothness"
    ),
    s!("curvature_smooth", ["in"], [], "smooth curvature from a normal map"),
    s!("height_blend", ["bg", "fg"], ["mask"], "blend two height maps: opacity contrast output"),
    s!("make_it_tile_photo", ["in"], [], "make a photo tile: width width_v"),
    s!("make_it_tile_patch", ["in"], [], "make an image tile with random patches"),
    s!("clamp", ["in"], [], "clamp values: low high apply_alpha"),
    s!("highpass", ["in"], [], "high-pass filter: radius"),
    s!("swirl", ["in"], [], "swirl distortion: amount radius"),
    s!("edge_wear", ["in"], ["noise"], "edge wear mask from a height map: amount width contrast"),
    s!("dirt", ["in"], ["noise"], "dirt mask from an ambient occlusion map: amount contrast"),
    s!("dust", ["in"], ["noise"], "dust mask from a normal map: amount contrast"),
    s!("output", [], ["in"], "graph output: usage identifier"),
    s!("dot", ["in"], [], "re-route a link"),
    s!("blur", ["in"], [], "gaussian blur: radius"),
    s!("directional_blur", ["in"], [], "angle length"),
    s!("sharpen", ["in"], [], "radius amount"),
    s!("levels", ["in"], [], "low high gamma out_low out_high"),
    s!("adjust", ["in"], [], "brightness contrast"),
    s!("invert", ["in"], [], "invert colours: invert"),
    s!("threshold", ["in"], [], "binary threshold: threshold softness mode(0 greater,1 greater or equal,2 lower,3 lower or equal)"),
    s!("histogram_scan", ["in"], [], "position width"),
    s!("quantize", ["in"], [], "steps"),
    s!("to_gray", ["in"], [], "luminance"),
    s!("channel", ["in"], [], "ch(0 r,1 g,2 b,3 a)"),
    s!("merge", [], ["r", "g", "b", "a"], "combine channels"),
    s!("tint", ["in"], [], "multiply by colour: r g b"),
    s!(
        "colorize",
        ["in"],
        [],
        "gradient map: interp(0 linear,1 smooth,2 step,3 smoother) smoothness addressing(0 clamp,1 repeat) use_alpha  stops p0 r0 g0 b0 a0 ... p15 r15 g15 b15 a15"
    ),
    s!("hsl", ["in"], [], "hue sat val"),
    s!("transform", ["in"], [], "offset_x offset_y scale_x scale_y rotation(turns)"),
    s!(
        "mirror",
        ["in"],
        [],
        "mirror an image: axis(0 x,1 y,2 both,3 corner) offset_x offset_y invert_x invert_y corner(0 top left,1 top right,2 bottom left,3 bottom right)"
    ),
    s!("warp", ["in", "warp"], [], "slope warp: intensity filtering(0 bilinear,1 nearest,2 bicubic)"),
    s!("directional_warp", ["in", "warp"], [], "warp in one direction: angle intensity offset filtering(0 bilinear,1 nearest,2 bicubic)"),
    s!(
        "blend",
        ["fg", "bg"],
        ["mask"],
        "blend two images: mode (copy, add, subtract, multiply, screen, overlay, max, min, difference, soft_light, color_dodge, color_burn, linear_burn, hard_light, exclusion, divide, linear_light, color, luminosity, dissolve, add_sub, switch, and as compiled graphs blend: add_compiled, subtract_compiled, add_sub_compiled, divide_compiled, soft_light_compiled) opacity alpha_mode(0 source alpha,1 ignore alpha,2 straight,3 premultiplied) crop_x0 crop_y0 crop_x1 crop_y1"
    ),
    s!("normal", ["in"], [], "height->normal (OpenGL +Y): strength directx alpha_from_input"),
    s!("normal_blend", ["a", "b"], ["mask"], "blend normal maps (a = background, b = foreground): opacity"),
    s!(
        "ao",
        ["in"],
        [],
        "ambient occlusion: mode(0 blur,1 horizon) radius intensity hbao_radius hbao_intensity quality(0 4,1 8,2 16 samples) use_world_units height_depth surface_size height_depth_cm"
    ),
    s!(
        "quantize_color",
        ["in"],
        [],
        "reduce to few colours (k-means): color_amount contour_smoothing dithering dithering_pattern(0 noise,1 bayer) ignore_alpha distance_color_space(0 lab,1 rgb)"
    ),
    s!("curvature", ["in"], [], "curvature of a normal map: mode(0 normal map,1 height map) intensity directx radius"),
    s!("edge_detect", ["in"], [], "edge lines: mode(0 lines,1 sobel) edge_width edge_roundness invert tolerance intensity"),
    s!("emboss", ["in"], [], "angle intensity"),
    s!("non_uniform_blur", ["in", "mask"], [], "blur by a mask: intensity"),
    s!("radial_blur", ["in"], [], "spin blur around a centre: mode(0 spin,1 zoom) samples angle x y amount radius"),
    s!("anisotropic_blur", ["in"], [], "blur stretched in one direction: intensity anisotropy angle"),
    s!(
        "symmetry",
        ["in"],
        [],
        "mirror the image around a line: axis(0 x,1 y,2 both,3 diagonal,4 anti-diagonal,5 eight-fold) offset flip transfer(0 copy,1 add,2 sub,3 mul,4 add sub,5 max,6 min)"
    ),
    s!("skew", ["in"], [], "shear that keeps tiling: skew_x skew_y (whole numbers) align(0 centre,1 top left,2 bottom right)"),
    s!("auto_levels", ["in"], [], "stretch values to the full range: per_channel"),
    s!("channel_mixer", ["in"], [], "mix colour channels: rr rg rb gr gg gb br bg bb mono"),
    s!("alpha_merge", ["rgb"], ["alpha"], "put a grayscale image into the alpha channel"),
    s!("color_to_mask", ["in"], [], "mask of pixels close to a colour: r g b range softness"),
    s!("replace_color", ["in"], [], "replace one colour with another: src_r src_g src_b dst_r dst_g dst_b range softness"),
    s!("pow", ["in"], [], "raise values to a power: exponent"),
    s!("histogram_shift", ["in"], [], "shift all values: position"),
    s!("grayscale_conversion", ["in"], [], "weighted grayscale: r g b"),
    s!(
        "cells",
        [],
        ["color"],
        "cellular noise: scale jitter seed variant(0 cells1,1 cells2,2 cells3,3 cells4) luminance_random color_source(0 random,1 pseudorandom,2 image) pseudorandom_seed edge_width hardness contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y"
    ),
    s!(
        "fractal_sum",
        [],
        [],
        "sum of noise octaves: scale min_level max_level roughness seed mode(0 base,1 turbulence,2 creased) contrast invert disorder disorder_speed"
    ),
    s!(
        "waveform",
        [],
        [],
        "tileable wave pattern: count angle warp warp_scale seed shape(0 sine,1 triangle,2 saw,3 square) contrast invert disorder disorder_speed"
    ),
    s!(
        "gaussian_spots",
        [],
        [],
        "soft gaussian spots: scale size size_variation seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y"
    ),
    s!("normal_invert", ["in"], [], "flip normal map channels: invert_x invert_y invert_z invert_a"),
    s!("normal_normalize", ["in"], [], "renormalise a normal map"),
    s!("normal_vector_rotation", ["in"], ["rotation"], "rotate normal vectors: angle directx"),
    s!("normal_to_height", ["in"], [], "integrate a normal map into a height map: directx relief_balance opacity"),
    s!("sobel", ["in"], [], "sobel edge magnitude: intensity"),
    s!("glow", ["in"], [], "glow around bright areas: width intensity clear glow_r glow_g glow_b"),
    s!("mosaic", ["in"], ["mosaic_map"], "mosaic of the map regions (or a pixel grid without a map): count intensity"),
    s!("non_uniform_directional_warp", ["in", "intensity"], ["angle"], "warp along a direction chosen per pixel: intensity angle"),
    s!(
        "multi_directional_warp",
        ["in", "warp"],
        [],
        "warp in several directions at once: intensity directions(0 one,1 two,2 four) angle mode(0 average,1 max,2 min,3 chain)"
    ),
    s!("normal_combine", ["base", "detail"], [], "combine two normal maps: technique(0 whiteout,1 channel mixer,2 detail oriented)"),
    s!(
        "triangle_grid",
        [],
        [],
        "grid of triangles: style(0 squares,1 jittered mesh) count border variation x_amount y_amount random_position_multiplier quincunx_offset_x quincunx_offset_y triangulation(0 fixed,1 alternate,2 random) output(0 smooth,1 flat,2 edge distance) height_random thin_lines distance_to_edge_thickness thin_line_color thin_line_background seed contrast invert"
    ),
    s!("grunge_map", [], [], "grunge map 1-15: map balance contrast brush_pattern cracks disorder coverage seed invert"),
    s!("rgba_split", ["in"], [], "split into R, G, B and A grayscale outputs"),
    s!("alpha_split", ["in"], [], "split into an RGB output and an alpha output"),
    s!("weave_generator", [], [], "woven threads with gaps: tile_x tile_y weave shape gap_x gap_y contrast invert"),
    s!("starburst", [], [], "star-like rays: count rays sharpness size rotation rotation_random seed contrast invert"),
    s!("gradient_reflected", [], [], "gradient that goes 0-1-0: count angle contrast invert"),
    s!("shape_stroke", ["in"], [], "outline of a shape: width offset softness threshold opacity color_r color_g color_b"),
    s!("shape_glow", ["in"], [], "glow around a shape: width softness intensity threshold opacity spread color_r color_g color_b"),
    s!("chrominance_extract", ["in"], [], "colour without brightness"),
    s!("shape_drop_shadow", ["in"], [], "drop shadow of a mask: angle distance blur opacity spread"),
];

pub fn spec(kind: &str) -> Option<&'static Spec> {
    SPECS
        .iter()
        .chain(crate::func::FUNC_SPECS.iter())
        .chain(crate::fxmap::FXM_SPECS.iter())
        .chain(crate::atomic::SPECS.iter())
        .chain(crate::compiled_ops::SPECS.iter())
        .chain(crate::patterns::SPECS.iter())
        .find(|s| s.kind == kind)
}

pub fn category(kind: &str) -> &'static str {
    match kind {
        "color" | "white_noise" | "gaussian_noise" | "perlin" | "fbm" | "clouds" | "voronoi" | "voronoi_fractal" | "bnw_spots" | "crystal"
        | "directional_noise" | "grunge" | "plasma" | "gradient" | "gradient_axial" | "gradient_circular" | "circle" | "rect" | "polygon"
        | "shape" | "star" | "checker" | "bricks" | "stripes" | "weave" | "tile_sampler" | "tile_generator" | "splatter" | "splatter_circular"
        | "pixel_processor" | "value_processor" | "fx_map" | "text" | "svg" | "cells" | "fractal_sum" | "waveform" | "gaussian_spots"
        | "triangle_grid" | "starburst" | "gradient_reflected" | "weave_generator" | "grunge_map" => "Generators",
        "sdf_shape" | "sdf_op" | "sdf_mask" | "sdf_form" => "SDF",
        "bitmap" | "input" | "input_value" | "subgraph" | "output" | "dot" => "Utility",
        k if k.starts_with("f_") => "Function",
        k if k.starts_with("fxm_") => "Pattern Program",
        "normal"
        | "normal_blend"
        | "normal_combine"
        | "normal_invert"
        | "normal_normalize"
        | "normal_vector_rotation"
        | "normal_to_height"
        | "ao"
        | "curvature"
        | "curvature_smooth"
        | "height_blend"
        | "edge_wear"
        | "dirt"
        | "dust" => "Material",
        _ => "Filters",
    }
}

/// Friendly display name for a node kind.
pub fn label(kind: &str) -> String {
    let named = match kind {
        "fbm" => "Fractal Noise",
        "clouds" => "Clouds",
        "bnw_spots" => "BnW Spots",
        "merge" => "RGBA Merge",
        "colorize" => "Gradient Map",
        "voronoi_fractal" => "Voronoi Fractal",
        "color" => "Uniform Color",
        "hsl" => "HSL",
        "ao" => "Ambient Occlusion",
        "transform" => "Transform 2D",
        "blur_hq" => "Blur HQ",
        "make_it_tile_photo" => "Make It Tile Photo",
        "make_it_tile_patch" => "Make It Tile Patch",
        "sdf_shape" => "SDF Shape",
        "sdf_op" => "SDF Combine",
        "sdf_mask" => "SDF Mask",
        "sdf_form" => "SDF 3D Form",
        "pixel_processor" => "Pixel Processor",
        "value_processor" => "Value Processor",
        "svg" => "SVG",
        "text" => "Text",
        "fx_map" => "Pattern Program Map",
        "input_value" => "Input Value",
        "normal" => "Normal From Height",
        "normal_blend" => "Normal Blend",
        "rgba_split" => "RGBA Split",
        "pow" => "Pow",
        "sobel" => "Sobel",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    let fnamed = match kind {
        "f_vec2" => "Float2",
        "f_vec3" => "Float3",
        "f_vec4" => "Float4",
        "f_ivec2" => "Integer2",
        "f_ivec3" => "Integer3",
        "f_ivec4" => "Integer4",
        "f_int" => "Integer",
        "f_bool" => "Boolean",
        "f_graph_input" => "Graph Input",
        "f_sample_gray" => "Sample Grayscale",
        "f_sample_color" => "Sample Color",
        "f_lerp" => "Lerp",
        "f_atan2" | "f_atan2_approx" => "Atan2",
        "f_eq" => "Equal",
        "f_ne" => "Not Equal",
        "f_lt" => "Less Than",
        "f_le" => "Less Or Equal",
        "f_gt" => "Greater Than",
        "f_ge" => "Greater Or Equal",
        "f_pos" => "Position",
        "f_out" => "Function Output",
        "fxm_out" => "Program Output",
        _ => "",
    };
    if !fnamed.is_empty() {
        return fnamed.to_string();
    }
    let kind = kind.strip_prefix("fxm_").or_else(|| kind.strip_prefix("f_")).unwrap_or(kind);
    kind.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Output usages in the order they are listed on a graph node.
pub const OUTPUT_USAGES: [&str; 12] = [
    "basecolor",
    "normal",
    "roughness",
    "metallic",
    "height",
    "ambient_occlusion",
    "opacity",
    "emissive",
    "diffuse",
    "specular",
    "glossiness",
    "out",
];

pub fn usage_label(usage: &str) -> String {
    match usage {
        "basecolor" => "Base Color",
        "normal" => "Normal",
        "roughness" => "Roughness",
        "metallic" => "Metallic",
        "height" => "Height",
        "ambient_occlusion" => "Ambient Occlusion",
        "opacity" => "Opacity",
        "emissive" => "Emissive",
        "diffuse" => "Diffuse",
        "specular" => "Specular",
        "glossiness" => "Glossiness",
        "out" => "Output",
        other => return label(other),
    }
    .to_string()
}

/// Value an unconnected Output node produces.
pub fn output_default(usage: &str) -> [f32; 4] {
    match usage {
        "normal" => [0.5, 0.5, 1.0, 1.0],
        "metallic" | "emissive" => [0.0, 0.0, 0.0, 1.0],
        "ambient_occlusion" | "opacity" => [1.0; 4],
        "specular" => [0.04, 0.04, 0.04, 1.0],
        _ => [0.5, 0.5, 0.5, 1.0],
    }
}

/// Small graph used for library thumbnails and the node smoke test. None = no preview graph.
pub fn demo_graph_for(kind: &str) -> Option<String> {
    if matches!(kind, "input" | "subgraph" | "bitmap" | "output" | "dot") {
        return None;
    }
    let sp = spec(kind)?;
    match kind {
        "fx_map" => return Some("size 64\nsubgraph dots\nmeta kind \"fxmap\"\nnode 1 fxm_marker\nnode 2 fxm_iterate body=@1 grid_x=4 grid_y=4\nnode 3 fxm_out in=@2\nend\nnode 1 fx_map program=\"dots\"\noutput o 1\n".into()),
        "value_processor" => return Some("size 64\nsubgraph avg\nmeta kind \"function\"\nnode 1 f_image_avg\nnode 2 f_out in=@1\nend\nnode 100 fbm scale=4 seed=2\nnode 1 value_processor fn=\"avg\" in=@100\noutput o 1\n".into()),
        "svg" => return Some("size 64\nresource art.svg PHN2ZyB2aWV3Qm94PSIwIDAgMTAgMTAiPjxjaXJjbGUgY3g9IjUiIGN5PSI1IiByPSI0IiBmaWxsPSIjZmZmIi8+PC9zdmc+\nnode 1 svg file=\"art.svg\"\noutput o 1\n".into()),
        _ => {}
    }
    let mut t = String::from(
        "size 64\nnode 100 fbm scale=4 octaves=4 seed=2\nnode 101 gradient angle=0.1\nnode 102 sdf_shape shape=0 size=0.3\nnode 103 sdf_shape shape=1 size=0.25 x=0.15\nnode 104 voronoi scale=5 mode=2 seed=3\nnode 105 threshold in=@104 threshold=0.2\nnode 106 flood_fill in=@105\nnode 107 normal in=@100 strength=3\nnode 108 colorize in=@100 p0=0 r0=1 g0=0.2 b0=0.1 p1=1 r1=0.1 g1=0.4 b1=1\n",
    )
    .replace("\\n", "\n");
    let (a, b) = match kind {
        "sdf_op" | "sdf_mask" => (102, 103),
        "flood_fill" => (105, 105),
        k if k.starts_with("flood_fill_") => (106, 106),
        "dust" | "curvature_smooth" | "normal_combine" | "normal_invert" | "normal_normalize" | "normal_vector_rotation" | "normal_to_height" => {
            (107, 107)
        }
        "vector_warp" => (100, 107),
        "chrominance_extract" => (108, 108),
        _ => (100, 101),
    };
    let mut line = format!("node 1 {kind}");
    match kind {
        "pixel_processor" => line += " expr=\"sin(u*18.849556)*0.5+0.5\"",
        "text" => line += " text=\"Ab\" size=0.5",
        "input_value" => line += " name=\"level\" type=0 v=0.5",
        "colorize" => line += " p0=0 r0=0.1 g0=0.1 b0=0.35 p1=1 r1=1 g1=0.8 b1=0.3",
        "merge" => line += " r=@100 g=@101 b=@100",
        "alpha_merge" => line += " alpha=@101",
        "color_to_mask" => line += " r=0.5 g=0.5 b=0.5 range=0.12 softness=0.2",
        "replace_color" => line += " src_r=0.5 src_g=0.5 src_b=0.5 range=0.2",
        "switch" => line += " a=@100 b=@101 index=1",
        _ => {}
    }
    for (i, slot) in sp.req.iter().enumerate() {
        line += &format!(" {slot}=@{}", if i % 2 == 0 { a } else { b });
    }
    t += &line;
    t += "\noutput o 1\n";
    Some(t)
}

/// Suffix of the text key that keeps the exact digits of an integer setting too large for `Node::nums`.
pub const EXACT_SUFFIX: &str = ".exact";

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub kind: String,
    pub nums: BTreeMap<String, f32>,
    pub strs: BTreeMap<String, String>,
    pub ins: BTreeMap<String, u32>,
    /// For links coming from a graph (subgraph) node: which of its outputs the slot reads. Absent = default output.
    pub in_out: BTreeMap<String, String>,
    pub pos: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct Comment {
    pub text: String,
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub color: u8,
}

#[derive(Debug, Clone)]
pub struct Exposed {
    pub label: String,
    pub node: u32,
    pub param: String,
    pub min: f32,
    pub max: f32,
}

/// Version of the graph text format this build writes. Files without a `format` line are version 1 (they load
/// unchanged); a file with a newer version is refused with a clear message instead of being half-understood.
/// Format 3 writes names where 1 and 2 wrote numbers (Output Format, blend mode, pattern shape, `f_random`'s generator)
/// and renames seven function ops; the older spellings still read the same (see [`crate::names`]).
pub const FORMAT_VERSION: u32 = 3;

/// What resolving a live link found (kept in memory only).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LinkInfo {
    /// the file that was actually read (the stored path, or a match found in the search folders)
    pub found: Option<std::path::PathBuf>,
    pub issue: Option<String>,
    /// subgraph names this link brought into the package
    pub members: Vec<String>,
}

/// A live link to a graph in another package file (`link <name> "<relative path>" ["<graph>"]`). The linked graph is
/// not stored in this file: it is read from disk each time the file is opened or the links are refreshed, and used
/// through a subgraph node whose `graph` is `<name>`. Every node that uses the same name is an instance of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub name: String,
    pub path: String,
    /// which graph of that package; `None` = its main graph
    pub graph: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Graph {
    /// format version the file was written with (1 when it had no `format` line)
    pub format: u32,
    pub links: Vec<Link>,
    /// subgraph names that came from resolving `links` (read-only copies, never written to the file)
    pub managed: BTreeSet<String>,
    /// what the last `resolve_links` found for every link (not saved): the file used and the problem, if any
    pub link_info: BTreeMap<String, LinkInfo>,
    pub size: usize,
    /// pixel height at the nominal size; 0 means square (same as `size`)
    pub height: usize,
    pub nodes: BTreeMap<u32, Node>,
    pub outputs: Vec<(String, u32)>,
    /// `output name id.pin` in a file: which named output of the node feeds the graph output (legacy output lines)
    pub output_pins: BTreeMap<String, String>,
    pub subgraphs: BTreeMap<String, Graph>,
    pub resources: BTreeMap<String, Vec<u8>>,
    pub exposed: Vec<Exposed>,
    pub comments: Vec<Comment>,
    pub seed_offset: u32,
    pub meta: BTreeMap<String, String>,
}

impl Default for Graph {
    fn default() -> Self {
        Graph {
            format: FORMAT_VERSION,
            links: Vec::new(),
            managed: BTreeSet::new(),
            link_info: BTreeMap::new(),
            size: 512,
            height: 0,
            nodes: BTreeMap::new(),
            outputs: Vec::new(),
            output_pins: BTreeMap::new(),
            subgraphs: BTreeMap::new(),
            resources: BTreeMap::new(),
            exposed: Vec::new(),
            comments: Vec::new(),
            seed_offset: 0,
            meta: BTreeMap::new(),
        }
    }
}

/// The output of node `src` that a link reads when it names `pin`: `None` for the default output (no pin, or the first).
fn pin_read(nodes: &BTreeMap<u32, Node>, src: u32, pin: Option<&String>) -> Option<String> {
    let kind = nodes.get(&src).map_or("", |n| n.kind.as_str());
    pin.filter(|p| !p.is_empty() && node_outputs(kind).first().is_none_or(|first| *first != p.as_str())).cloned()
}

fn split_tokens(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut had_q = false;
    for c in line.chars() {
        match c {
            '"' => {
                in_q = !in_q;
                had_q = true;
            }
            c if c.is_whitespace() && !in_q => {
                if !cur.is_empty() || had_q {
                    out.push(std::mem::take(&mut cur));
                    had_q = false;
                }
            }
            c => cur.push(c),
        }
    }
    if in_q {
        return Err("unterminated quote".into());
    }
    if !cur.is_empty() || had_q {
        out.push(cur);
    }
    Ok(out)
}

// ---------- base64 (for embedded resources) ----------
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(d: &[u8]) -> String {
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

pub fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes() {
        if ch == b'=' {
            break;
        }
        let v = B64.iter().position(|&b| b == ch).ok_or_else(|| "invalid base64".to_string())? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

pub const THUMB: usize = 96;

pub struct EvalAll {
    pub thumbs: HashMap<u32, Vec<u8>>,
    pub errors: HashMap<u32, String>,
    pub kept: HashMap<u32, Arc<Tex>>,
    /// output size of every evaluated node (differs from the graph size when a node has its own Output Size)
    pub sizes: HashMap<u32, usize>,
}

/// A validated, immutable graph and the compiled functions shared by its instances.
pub struct PreparedGraph {
    graph: Graph,
    programs: Mutex<HashMap<String, Result<Arc<crate::func::Prog>, String>>>,
    dependencies: OnceLock<BTreeMap<String, BTreeSet<String>>>,
}

impl PreparedGraph {
    pub fn parse(text: &str) -> Result<Self, Error> {
        Ok(Self { graph: Graph::parse(text)?, programs: Mutex::default(), dependencies: OnceLock::new() })
    }

    pub fn instance(&self, seed: u32) -> GraphInstance<'_> {
        GraphInstance { plan: self, nodes: BTreeMap::new(), seed }
    }

    /// Values that can affect this output via a reachable node, function, or pattern.
    pub fn output_parameters(&self, output: &str) -> Option<&BTreeSet<String>> {
        self.dependencies
            .get_or_init(|| self.graph.output_nodes().into_iter().map(|(_, name, root)| (name, output_parameters(&self.graph, root))).collect())
            .get(output)
    }

    fn program(&self, name: &str) -> Result<Arc<crate::func::Prog>, String> {
        if let Some(program) = self.programs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(name) {
            return program.clone();
        }
        let program = crate::func::compile_named(&self.graph.subgraphs, name).map(Arc::new);
        self.programs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(name.to_owned(), program.clone());
        program
    }
}

fn output_parameters(graph: &Graph, root: u32) -> BTreeSet<String> {
    let reachable = reached(graph, &[root]);
    let mut parameters: BTreeSet<_> = graph.exposed.iter().filter(|p| reachable.contains(&p.node)).map(|p| p.label.clone()).collect();
    let mut pending: Vec<&Node> = reachable.iter().filter_map(|id| graph.nodes.get(id)).collect();
    let mut visited = HashSet::new();
    while let Some(node) = pending.pop() {
        if matches!(node.kind.as_str(), "input_value" | "f_graph_input")
            && let Some(name) = node.strs.get("name")
        {
            parameters.insert(name.clone());
        }
        for (_, name) in node.strs.iter().filter(|(slot, _)| matches!(slot.as_str(), "fn" | "program" | "graph") || slot.starts_with("fn.")) {
            if visited.insert(name)
                && let Some(inner) = graph.subgraphs.get(name)
            {
                pending.extend(inner.nodes.values());
            }
        }
    }
    parameters
}

impl std::ops::Deref for PreparedGraph {
    type Target = Graph;

    fn deref(&self) -> &Graph {
        &self.graph
    }
}

/// The runtime parameter bindings of a single material instance.
pub struct GraphInstance<'a> {
    plan: &'a PreparedGraph,
    nodes: BTreeMap<u32, Node>,
    seed: u32,
}

impl GraphInstance<'_> {
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.plan.nodes.iter().map(|(id, node)| self.nodes.get(id).unwrap_or(node))
    }

    pub fn work_nodes(&self, outputs: &[&str]) -> usize {
        self.reachable_nodes(outputs).len()
    }

    /// External images the selected outputs can read. A computed input index can read any slot.
    pub fn image_inputs(&self, outputs: &[&str]) -> Option<BTreeSet<u32>> {
        self.reachable_nodes(outputs)
            .into_iter()
            .filter_map(|id| self.nodes.get(&id).or_else(|| self.plan.nodes.get(&id)))
            .filter(|node| node.kind == "input")
            .map(|node| (!node.strs.contains_key("fn.idx")).then(|| node.nums.get("idx").copied().unwrap_or(0.0).max(0.0) as u32))
            .collect()
    }

    fn reachable_nodes(&self, outputs: &[&str]) -> HashSet<u32> {
        let roots: Vec<_> =
            self.plan.output_nodes().into_iter().filter(|(_, name, _)| outputs.contains(&name.as_str())).map(|(_, _, id)| id).collect();
        reached(self.plan, &roots)
    }

    pub fn bind_number(&mut self, id: u32, slot: &str, value: f32) {
        if let Some(node) = self.plan.nodes.get(&id) {
            self.nodes.entry(id).or_insert_with(|| node.clone()).nums.insert(slot.to_owned(), value);
        }
    }

    pub fn bind_text(&mut self, id: u32, value: &str) {
        if let Some(node) = self.plan.nodes.get(&id).filter(|node| node.kind == "input_value") {
            self.nodes.entry(id).or_insert_with(|| node.clone()).strs.insert("v".into(), value.to_owned());
        }
    }

    pub fn evaluate_owned(&self, ec: &EvalContext, size: usize, images: Vec<Option<Tex>>, outputs: &[&str]) -> Result<BTreeMap<String, Tex>, Error> {
        crate::context::scoped(ec, || {
            let ext = images.into_iter().map(|t| t.map(Val::of)).collect();
            self.plan.evaluate_vals(ec, Some(size), ext, Some((self, outputs)))
        })
    }
}

struct Ctx<'a> {
    resident: bool,
    size: usize,
    /// pixel height (equals `size` for a square graph)
    height: usize,
    /// the graph's own (nominal) size: absolute node sizes are scaled by size / nominal so quick previews stay proportional
    nominal: usize,
    /// in a compiled graph, on a node whose size isn't the graph's: the graph's size, which `$outputsize` reads everywhere in the node
    graph: Option<[usize; 2]>,
    lib: &'a BTreeMap<String, Graph>,
    res: &'a BTreeMap<String, Vec<u8>>,
    so: u32,
    depth: usize,
    /// values of the Input Value nodes of the graph being evaluated (read by function graphs)
    gin: &'a BTreeMap<String, crate::func::Value>,
    instance: Option<&'a GraphInstance<'a>>,
    plan: Option<&'a PreparedGraph>,
    /// what the host gave this evaluation: its cancel flag, logging, profiler and memory counter
    ec: &'a EvalContext,
}

impl Ctx<'_> {
    fn program(&self, name: &str) -> Result<Arc<crate::func::Prog>, String> {
        match self.plan {
            Some(plan) => plan.program(name),
            None => crate::func::compile_named(self.lib, name).map(Arc::new),
        }
    }
}

/// Node id used for the helper nodes of [`Graph::preview_host`] (far above any id an author would reach).
pub const PREVIEW_ID: u32 = 4_000_000_001;

impl Graph {
    /// "texture", "function" or "fxmap".
    pub fn graph_kind(&self) -> &str {
        match self.meta.get("kind").map(String::as_str) {
            Some("function") => "function",
            Some("fxmap") => "fxmap",
            _ => "texture",
        }
    }

    /// Function graphs and pattern programs are not textures. To look at one, wrap it in a texture graph that
    /// runs it: a pixel processor for a function, a pattern map for a program. Returns the wrapper and the id of the
    /// node that shows the result (`None` for ordinary graphs).
    pub fn preview_host(&self) -> Option<(Graph, u32)> {
        let kind = self.graph_kind();
        if kind == "texture" {
            return None;
        }
        let mut host = Graph {
            size: self.size,
            height: self.height,
            seed_offset: self.seed_offset,
            resources: self.resources.clone(),
            subgraphs: self.subgraphs.clone(),
            ..Graph::default()
        };
        let mut me = self.clone();
        me.subgraphs.clear();
        me.resources.clear();
        host.subgraphs.insert("__self".into(), me);
        let mut n = Node::default();
        if kind == "function" {
            n.kind = "pixel_processor".into();
            n.strs.insert("fn".into(), "__self".into());
        } else {
            n.kind = "fx_map".into();
            n.strs.insert("program".into(), "__self".into());
            n.nums.insert("color".into(), 0.0);
        }
        host.nodes.insert(PREVIEW_ID, n);
        let mut o = Node { kind: "output".into(), ..Node::default() };
        o.strs.insert("usage".into(), "basecolor".into());
        o.ins.insert("in".into(), PREVIEW_ID);
        host.nodes.insert(PREVIEW_ID + 1, o);
        Some((host, PREVIEW_ID))
    }

    pub fn parse(text: &str) -> Result<Graph, Error> {
        let g = Graph::parse_unchecked(text)?;
        g.validate()?;
        Ok(g)
    }

    /// Parse without validating connections (used by the editor for work-in-progress graphs).
    pub fn parse_unchecked(text: &str) -> Result<Graph, Error> {
        let mut g = Graph { format: 1, ..Graph::default() };
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while let Some(raw) = lines.get(i).copied() {
            let ln = i + 1;
            i += 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix("subgraph ") {
                let mut body = String::new();
                let mut closed = false;
                while let Some(l) = lines.get(i).copied() {
                    i += 1;
                    if l.trim() == "end" {
                        closed = true;
                        break;
                    }
                    body.push_str(l);
                    body.push('\n');
                }
                if !closed {
                    return Err(Error::Parse(ln, "subgraph without matching 'end'".into()));
                }
                let sub = Graph::parse_unchecked(&body).map_err(|e| Error::Parse(ln, format!("in subgraph '{}': {e}", name.trim())))?;
                g.subgraphs.insert(name.trim().to_string(), sub);
                continue;
            }
            if let Some(rest) = line.strip_prefix("comment ") {
                let t: Vec<&str> = rest.split_whitespace().collect();
                let [x, y, w, h, color, text @ ..] = t.as_slice() else {
                    return Err(Error::Parse(ln, "usage: comment x y w h color text...".into()));
                };
                let f = |v: &str| v.parse::<f32>().map_err(|_| Error::Parse(ln, "bad comment number".into()));
                g.comments.push(Comment {
                    text: text.join(" "),
                    pos: [f(x)?, f(y)?],
                    size: [f(w)?.max(40.0), f(h)?.max(30.0)],
                    color: color.parse().unwrap_or(0),
                });
                continue;
            }
            let t = split_tokens(line).map_err(|m| Error::Parse(ln, m))?;
            // (a line that isn't empty has a first token)
            let word = |k: usize| t.get(k).map_or("", String::as_str);
            match word(0) {
                "format" => {
                    let v = t.get(1).and_then(|s| s.parse::<u32>().ok()).ok_or_else(|| Error::Parse(ln, "format needs an integer".into()))?;
                    if v > FORMAT_VERSION {
                        return Err(Error::Parse(
                            ln,
                            format!("this file uses format {v}, but this version reads up to format {FORMAT_VERSION}; please update"),
                        ));
                    }
                    g.format = v;
                }
                "link" => {
                    if t.len() < 3 || t.len() > 4 {
                        return Err(Error::Parse(ln, "usage: link <name> \"<path>\" [\"<graph>\"]".into()));
                    }
                    g.links.push(Link { name: word(1).to_string(), path: word(2).to_string(), graph: t.get(3).cloned().filter(|s| !s.is_empty()) });
                }
                "size" => {
                    g.size = t.get(1).and_then(|s| s.parse::<usize>().ok()).ok_or_else(|| Error::Parse(ln, "size needs an integer".into()))?;
                    g.height = match t.get(2) {
                        Some(h) => h.parse::<usize>().map_err(|_| Error::Parse(ln, "size height needs an integer".into()))?,
                        None => 0,
                    };
                }
                "seed_offset" => {
                    g.seed_offset =
                        t.get(1).and_then(|s| s.parse::<u32>().ok()).ok_or_else(|| Error::Parse(ln, "seed_offset needs an integer".into()))?;
                }
                "meta" => {
                    if t.len() != 3 {
                        return Err(Error::Parse(ln, "usage: meta <key> \"value\"".into()));
                    }
                    g.meta.insert(word(1).to_string(), word(2).to_string());
                }
                "resource" => {
                    if t.len() != 3 {
                        return Err(Error::Parse(ln, "usage: resource <name> <base64>".into()));
                    }
                    g.resources.insert(word(1).to_string(), b64_decode(word(2)).map_err(|m| Error::Parse(ln, m))?);
                }
                "expose" => {
                    if t.len() != 6 {
                        return Err(Error::Parse(ln, "usage: expose <label> <node> <param> <min> <max>".into()));
                    }
                    let p = |s: &str| s.parse::<f32>().map_err(|_| Error::Parse(ln, "bad number".into()));
                    g.exposed.push(Exposed {
                        label: word(1).to_string(),
                        node: word(2).parse().map_err(|_| Error::Parse(ln, "bad node id".into()))?,
                        param: word(3).to_string(),
                        min: p(word(4))?,
                        max: p(word(5))?,
                    });
                }
                "output" => {
                    if t.len() != 3 {
                        return Err(Error::Parse(ln, "usage: output <name> <node-id>".into()));
                    }
                    let (idtxt, pin) = match word(2).split_once('.') {
                        Some((a, b)) => (a, Some(b.to_string())),
                        None => (word(2), None),
                    };
                    let id = idtxt.parse::<u32>().map_err(|_| Error::Parse(ln, "bad node id".into()))?;
                    g.outputs.push((word(1).to_string(), id));
                    if let Some(pin) = pin {
                        g.output_pins.insert(word(1).to_string(), pin);
                    }
                }
                "node" => {
                    if t.len() < 3 {
                        return Err(Error::Parse(ln, "usage: node <id> <type> [key=value ...]".into()));
                    }
                    let id = word(1).parse::<u32>().map_err(|_| Error::Parse(ln, "bad node id".into()))?;
                    if g.nodes.contains_key(&id) {
                        return Err(Error::Parse(ln, format!("duplicate node id {id}")));
                    }
                    let mut n = Node { kind: word(2).to_string(), ..Default::default() };
                    for kv in t.iter().skip(3) {
                        let (k, v) = kv.split_once('=').ok_or_else(|| Error::Parse(ln, format!("expected key=value, got '{kv}'")))?;
                        if let Some(r) = v.strip_prefix('@') {
                            let (idpart, outpart) = match r.split_once('.') {
                                Some((a, b)) => (a, Some(b)),
                                None => (r, None),
                            };
                            n.ins.insert(k.to_string(), idpart.parse::<u32>().map_err(|_| Error::Parse(ln, format!("bad reference '{v}'")))?);
                            if let Some(o) = outpart.filter(|o| !o.is_empty()) {
                                n.in_out.insert(k.to_string(), o.to_string());
                            }
                        } else if let Some(exact) = k.strip_suffix(EXACT_SUFFIX) {
                            n.strs.insert(format!("{exact}{EXACT_SUFFIX}"), v.to_string());
                        } else if let Ok(x) = v.parse::<f32>() {
                            // an integer single precision can't hold (a seed of -1331112700) also keeps its digits, for functions
                            // that compute in integers
                            if let Ok(i) = v.parse::<i64>()
                                && x as i64 != i
                            {
                                n.strs.insert(format!("{k}{EXACT_SUFFIX}"), v.to_string());
                            }
                            match k {
                                "_x" => n.pos[0] = x,
                                "_y" => n.pos[1] = x,
                                _ => {
                                    n.nums.insert(k.to_string(), x);
                                }
                            }
                        } else {
                            n.strs.insert(k.to_string(), v.to_string());
                        }
                    }
                    crate::names::read_node(&mut n).map_err(|m| Error::Parse(ln, m))?;
                    g.nodes.insert(id, n);
                }
                other => return Err(Error::Parse(ln, format!("unknown directive '{other}'"))),
            }
        }
        // `output name id.pin` needs a real Output node to carry the pin
        if !g.output_pins.is_empty() {
            g.migrate_outputs();
        }
        Ok(g)
    }

    /// All outputs: (usage, identifier, node id). Includes legacy `output` lines.
    pub fn output_nodes(&self) -> Vec<(String, String, u32)> {
        let mut v = Vec::new();
        for (id, n) in &self.nodes {
            if n.kind == "output" {
                let usage = n.strs.get("usage").cloned().unwrap_or_else(|| "basecolor".into());
                let ident = n.strs.get("identifier").filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| usage.clone());
                v.push((usage, ident, *id));
            }
        }
        for (name, id) in &self.outputs {
            v.push((name.clone(), name.clone(), *id));
        }
        v
    }

    /// Names of the outputs in their canonical order (base color, normal, roughness, ...).
    pub fn output_idents(&self) -> Vec<String> {
        let mut v: Vec<(usize, String)> =
            self.output_nodes().into_iter().map(|(u, i, _)| (OUTPUT_USAGES.iter().position(|x| *x == u).unwrap_or(99), i)).collect();
        v.sort();
        v.dedup();
        v.into_iter().map(|x| x.1).collect()
    }

    /// Convert legacy `output` assignments into real Output nodes.
    pub fn migrate_outputs(&mut self) {
        let legacy = std::mem::take(&mut self.outputs);
        let pins = std::mem::take(&mut self.output_pins);
        let first = self.nodes.keys().max().copied().unwrap_or(0) + 1;
        for (next, (name, id)) in (first..).zip(legacy) {
            let pos = self.nodes.get(&id).map(|n| [n.pos[0] + 150.0, n.pos[1]]).unwrap_or([0.0, 0.0]);
            let mut node = Node { kind: "output".into(), pos, ..Default::default() };
            node.strs.insert("usage".into(), name.clone());
            node.ins.insert("in".into(), id);
            if let Some(pin) = pins.get(&name) {
                node.in_out.insert("in".into(), pin.clone());
            }
            self.nodes.insert(next, node);
        }
    }

    /// Precision every node of this graph is quantised to unless it sets its own Output Format:
    /// 0 = 32-bit float (default, nothing is rounded), 1 = 8 bit, 2 = 16 bit, 3 = 16-bit float.
    pub fn default_format(&self) -> u32 {
        self.meta.get("ofmt").and_then(|s| s.parse::<u32>().ok()).filter(|v| *v <= 3).unwrap_or(0)
    }

    /// true when width and height are the same
    pub fn is_square(&self) -> bool {
        self.height == 0 || self.height == self.size
    }

    /// Height in pixels of an evaluation whose width is `size` (keeps the graph's aspect ratio).
    pub fn height_for(&self, size: usize) -> usize {
        if self.is_square() { size } else { ((size as f64 * self.height as f64 / self.size.max(1) as f64).round() as usize).clamp(1, 8192) }
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.size == 0 || self.size > 8192 {
            return Err(Error::BadSize(self.size));
        }
        if self.height > 8192 {
            return Err(Error::BadSize(self.height));
        }
        if self.output_nodes().is_empty() {
            return Err(Error::NoOutputs);
        }
        self.validate_nodes()?;
        for (_, name, id) in &self.output_nodes() {
            if !self.nodes.contains_key(id) {
                return Err(Error::MissingNode(0, *id));
            }
            if name.is_empty() || name.contains(['/', '\\', '.']) {
                return Err(Error::Parse(0, format!("invalid output name '{name}'")));
            }
        }
        self.check_cycles()
    }

    fn validate_nodes(&self) -> Result<(), Error> {
        for (id, n) in &self.nodes {
            let sp = spec(&n.kind).ok_or_else(|| Error::UnknownNode(*id, n.kind.clone()))?;
            for r in sp.req {
                if !n.ins.contains_key(*r) {
                    return Err(Error::MissingInput(*id, r.to_string()));
                }
            }
            for (slot, rid) in &n.ins {
                if !sp.req.contains(&slot.as_str()) && !sp.opt.contains(&slot.as_str()) {
                    return Err(Error::UnknownSlot(*id, slot.clone()));
                }
                if !self.nodes.contains_key(rid) {
                    return Err(Error::MissingNode(*id, *rid));
                }
                if let (Some(o), Some(src)) = (n.in_out.get(slot), self.nodes.get(rid)) {
                    let outs = node_outputs(&src.kind);
                    if !outs.is_empty() && !outs.contains(&o.as_str()) {
                        return Err(Error::BadParam(
                            *id,
                            format!("{slot}: node {rid} ({}) has no output '{o}' (outputs: {})", src.kind, outs.join(", ")),
                        ));
                    }
                }
            }
            if n.kind == "pixel_processor" {
                if !n.strs.contains_key("expr") && !n.strs.contains_key("fn") {
                    return Err(Error::BadParam(*id, "pixel_processor needs expr=\"...\" or fn=\"function\"".into()));
                }
                for (k, v) in &n.strs {
                    if k.starts_with("expr") {
                        expr::parse(v).map_err(|m| Error::BadParam(*id, format!("{k}: {m}")))?;
                    }
                }
            }
            if n.kind == "subgraph" {
                match n.strs.get("graph") {
                    Some(name) if self.subgraphs.contains_key(name) || self.links.iter().any(|l| l.name == *name) => {}
                    Some(name) => return Err(Error::BadParam(*id, format!("unknown subgraph '{name}'"))),
                    None => return Err(Error::BadParam(*id, "subgraph node needs graph=\"name\"".into())),
                }
            }
            if n.kind == "bitmap" && !n.strs.contains_key("file") {
                return Err(Error::BadParam(*id, "bitmap needs file=\"path.png\"".into()));
            }
        }
        Ok(())
    }

    /// Cycle detection (iterative DFS, 0=unseen 1=active 2=done).
    pub fn check_cycles(&self) -> Result<(), Error> {
        let mut state: HashMap<u32, u8> = HashMap::new();
        for &start in self.nodes.keys() {
            if state.get(&start).copied().unwrap_or(0) != 0 {
                continue;
            }
            let ins_of = |id: u32| -> Vec<u32> { self.nodes.get(&id).map(|n| n.ins.values().copied().collect()).unwrap_or_default() };
            let mut stack: Vec<(u32, Vec<u32>)> = vec![(start, ins_of(start))];
            state.insert(start, 1);
            while let Some((id, deps)) = stack.last_mut() {
                if let Some(d) = deps.pop() {
                    if !self.nodes.contains_key(&d) {
                        continue;
                    }
                    match state.get(&d).copied().unwrap_or(0) {
                        1 => return Err(Error::Cycle(d)),
                        0 => {
                            state.insert(d, 1);
                            stack.push((d, ins_of(d)));
                        }
                        _ => {}
                    }
                } else {
                    state.insert(*id, 2);
                    stack.pop();
                }
            }
        }
        Ok(())
    }

    pub fn to_text(&self) -> String {
        let mut s = format!("format {FORMAT_VERSION}\n");
        s.push_str(&if self.is_square() { format!("size {}\n", self.size) } else { format!("size {} {}\n", self.size, self.height) });
        for l in &self.links {
            match &l.graph {
                Some(gn) => s.push_str(&format!("link {} \"{}\" \"{}\"\n", l.name, l.path.replace('"', ""), gn.replace('"', ""))),
                None => s.push_str(&format!("link {} \"{}\"\n", l.name, l.path.replace('"', ""))),
            }
        }
        if self.seed_offset != 0 {
            s.push_str(&format!("seed_offset {}\n", self.seed_offset));
        }
        for (k, v) in &self.meta {
            if !v.is_empty() {
                s.push_str(&format!("meta {k} \"{}\"\n", v.replace(['"', '#'], "").replace('\n', " ")));
            }
        }
        for (k, v) in &self.resources {
            s.push_str(&format!("resource {k} {}\n", b64_encode(v)));
        }
        for (name, sub) in &self.subgraphs {
            if self.managed.contains(name) {
                continue;
            }
            s.push_str(&format!("subgraph {name}\n{}end\n", sub.to_text()));
        }
        for (id, n) in &self.nodes {
            s.push_str(&format!("node {id} {}", n.kind));
            if n.pos != [0.0, 0.0] {
                s.push_str(&format!(" _x={} _y={}", n.pos[0], n.pos[1]));
            }
            for (k, v) in &n.nums {
                match crate::names::written_name(&n.kind, k, *v) {
                    Some((key, name)) => s.push_str(&format!(" {key}=\"{name}\"")),
                    None => s.push_str(&format!(" {k}={v}")),
                }
            }
            for (k, v) in &n.strs {
                s.push_str(&format!(" {k}=\"{}\"", v.replace(['"', '#'], "")));
            }
            for (k, v) in &n.ins {
                match n.in_out.get(k).filter(|o| !o.is_empty() && !o.contains(|c: char| c.is_whitespace() || matches!(c, '"' | '.' | '#' | '='))) {
                    Some(o) => s.push_str(&format!(" {k}=@{v}.{o}")),
                    None => s.push_str(&format!(" {k}=@{v}")),
                }
            }
            s.push('\n');
        }
        for (name, id) in &self.outputs {
            s.push_str(&format!("output {name} {id}\n"));
        }
        for e in &self.exposed {
            s.push_str(&format!("expose \"{}\" {} {} {} {}\n", e.label.replace('"', ""), e.node, e.param, e.min, e.max));
        }
        for c in &self.comments {
            s.push_str(&format!(
                "comment {} {} {} {} {} {}\n",
                c.pos[0],
                c.pos[1],
                c.size[0],
                c.size[1],
                c.color,
                c.text.replace('#', "").replace('\n', " ")
            ));
        }
        s
    }

    /// Flatten into a self-contained package: embeds bitmaps, drops editor-only data.
    pub fn publish(&self) -> Result<String, Error> {
        self.validate()?;
        let mut g = self.clone();
        g.comments.clear();
        let mut res = std::mem::take(&mut g.resources);
        let mut seen: HashMap<String, String> = HashMap::new();
        fn embed(
            g: &mut Graph,
            res: &mut BTreeMap<String, Vec<u8>>,
            seen: &mut HashMap<String, String>,
            own: &BTreeMap<String, Vec<u8>>,
        ) -> Result<(), Error> {
            for (id, n) in g.nodes.iter_mut() {
                n.pos = [0.0, 0.0];
                if n.kind != "bitmap" {
                    continue;
                }
                let Some(f) = n.strs.get("file").cloned() else { continue };
                if let Some(k) = seen.get(&f) {
                    n.strs.insert("file".into(), k.clone());
                    continue;
                }
                let bytes = match res.get(&f).or_else(|| own.get(&f)) {
                    Some(b) => b.clone(),
                    None => read_disk(std::path::Path::new(&f)).map_err(|e| Error::BadParam(*id, format!("cannot embed '{f}': {e}")))?,
                };
                let base = std::path::Path::new(&f).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "bitmap.png".into());
                let mut key = base.clone();
                let mut c = 1;
                while res.contains_key(&key) && res.get(&key) != Some(&bytes) {
                    key = format!("{c}_{base}");
                    c += 1;
                }
                res.insert(key.clone(), bytes);
                seen.insert(f, key.clone());
                n.strs.insert("file".into(), key);
            }
            Ok(())
        }
        let own = BTreeMap::new();
        embed(&mut g, &mut res, &mut seen, &own)?;
        for sub in g.subgraphs.values_mut() {
            embed(sub, &mut res, &mut seen, &own)?;
        }
        g.resources = res;
        Ok(format!("# Published asset\n{}", g.to_text()))
    }

    /// Apply exposed-parameter overrides by label.
    pub fn apply_params(&mut self, p: &BTreeMap<String, f32>) -> Result<(), String> {
        for (label, v) in p {
            let e = self.exposed.iter().find(|e| &e.label == label).cloned().ok_or_else(|| format!("no exposed parameter '{label}'"))?;
            let n = self.nodes.get_mut(&e.node).ok_or_else(|| format!("exposed node {} missing", e.node))?;
            n.nums.insert(e.param.clone(), v.clamp(e.min.min(e.max), e.min.max(e.max)));
        }
        Ok(())
    }

    /// Import a whole (already made) graph as a reusable subgraph named `name`, so it can be placed as a node in this
    /// graph. Its own subgraphs are merged under a `name__` prefix and its embedded resources are carried over.
    pub fn import_graph(&mut self, mut other: Graph, name: &str) -> Result<(), String> {
        if self.subgraphs.contains_key(name) {
            return Err(format!("subgraph '{name}' already exists"));
        }
        if other.nodes.is_empty() {
            return Err("the graph is empty".into());
        }
        if other.output_nodes().is_empty() {
            return Err("the graph has no Output node to use as the node's result".into());
        }
        other.check_cycles().map_err(|e| e.to_string())?;
        let mut inner = std::mem::take(&mut other.subgraphs);
        let renamed: Vec<String> = inner.keys().cloned().collect();
        for g in std::iter::once(&mut other).chain(inner.values_mut()) {
            for n in g.nodes.values_mut() {
                let is_subgraph = n.kind == "subgraph";
                // the settings that name a graph: a subgraph node's `graph`, and the `fn`, `fn.*` and `program` of the
                // nodes that run a function or a program
                for (slot, r) in n.strs.iter_mut() {
                    let names_graph = match slot.as_str() {
                        "graph" => is_subgraph,
                        "fn" | "program" => true,
                        _ => slot.starts_with("fn."),
                    };
                    if names_graph && renamed.contains(r) {
                        *r = format!("{name}__{r}");
                    }
                }
            }
        }
        for (k, g) in inner {
            self.subgraphs.insert(format!("{name}__{k}"), g);
        }
        for (k, v) in std::mem::take(&mut other.resources) {
            self.resources.entry(k).or_insert(v);
        }
        self.subgraphs.insert(name.to_string(), other);
        Ok(())
    }

    /// True when subgraph `name` is a read-only copy that came from a live link.
    pub fn is_linked(&self, name: &str) -> bool {
        self.managed.contains(name)
    }

    /// (Re)read every live link relative to `base` (the folder of this file). Previous results are dropped first, so
    /// calling it again picks up changes made to the linked files. Returns one message per link that could not be
    /// used (missing file, parse error, unknown graph, empty graph, name clash); the other links still work.
    pub fn resolve_links(&mut self, base: &std::path::Path) -> Vec<String> {
        self.resolve_links_depth(base, 0)
    }

    fn resolve_links_depth(&mut self, base: &std::path::Path, depth: usize) -> Vec<String> {
        self.link_info.clear();
        for m in std::mem::take(&mut self.managed) {
            self.subgraphs.remove(&m);
        }
        let mut issues = Vec::new();
        if self.links.is_empty() {
            return issues;
        }
        if depth >= 8 {
            issues.push("links are nested too deeply (a package links itself?)".to_string());
            return issues;
        }
        for l in self.links.clone() {
            let start = issues.len();
            match self.resolve_one(&l, base, depth, &mut issues) {
                Some((path, members)) => {
                    self.link_info.insert(l.name.clone(), LinkInfo { found: Some(path), issue: None, members });
                }
                None => {
                    let msg = issues.get(start).cloned();
                    self.link_info.insert(l.name.clone(), LinkInfo { found: None, issue: msg, members: Vec::new() });
                }
            }
        }
        issues
    }

    /// Folders (relative to the package) that are searched by file name when a linked file is not where the link says.
    pub fn search_paths(&self) -> Vec<String> {
        self.meta.get("search_paths").map(|s| s.split(';').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()).unwrap_or_default()
    }

    fn resolve_one(&mut self, l: &Link, base: &std::path::Path, depth: usize, issues: &mut Vec<String>) -> Option<(std::path::PathBuf, Vec<String>)> {
        let stored = base.join(l.path.replace('\\', "/"));
        // a moved file is looked for by name in the package's search folders
        let path = if stored.is_file() {
            stored
        } else {
            self.search_paths()
                .iter()
                .map(|d| base.join(d.replace('\\', "/")).join(stored.file_name().unwrap_or_default()))
                .find(|c| c.is_file())
                .unwrap_or(stored)
        };
        if self.subgraphs.contains_key(&l.name) {
            issues.push(format!("link '{}': a graph with that name is already embedded", l.name));
            return None;
        }
        let text = match read_disk_text(&path) {
            Ok(t) => t,
            Err(e) => {
                issues.push(format!("link '{}': cannot read {} ({e})", l.name, path.display()));
                return None;
            }
        };
        let mut other = match Graph::parse_unchecked(&text) {
            Ok(g) => g,
            Err(e) => {
                issues.push(format!("link '{}': {} is not a valid package ({e})", l.name, path.display()));
                return None;
            }
        };
        other.migrate_outputs();
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| base.to_path_buf());
        issues.extend(other.resolve_links_depth(&dir, depth + 1));
        let main = match &l.graph {
            Some(gn) => match other.subgraphs.remove(gn) {
                Some(mut m) => {
                    // the rest of the package is what that graph may use
                    m.subgraphs = std::mem::take(&mut other.subgraphs);
                    m.resources.extend(std::mem::take(&mut other.resources));
                    m
                }
                None => {
                    issues.push(format!("link '{}': {} has no graph '{gn}'", l.name, path.display()));
                    return None;
                }
            },
            None => other,
        };
        let before: BTreeSet<String> = self.subgraphs.keys().cloned().collect();
        let mut members = Vec::new();
        if let Err(e) = self.import_graph(main, &l.name) {
            issues.push(format!("link '{}': {e}", l.name));
            return None;
        }
        for (k, sg) in self.subgraphs.iter_mut() {
            if !before.contains(k) {
                self.managed.insert(k.clone());
                members.push(k.clone());
                // keeps linked copies out of the Explorer's graph list
                sg.meta.insert("embedded".into(), "1".into());
            }
        }
        Some((path, members))
    }

    /// Turn a live link into an ordinary embedded copy: the linked graphs become part of this package and are saved
    /// with it. Instances keep working; later changes to the linked file no longer reach them.
    pub fn break_link(&mut self, name: &str) -> Result<usize, String> {
        let Some(i) = self.links.iter().position(|l| l.name == name) else { return Err(format!("no link '{name}'")) };
        let info = self.link_info.remove(name).unwrap_or_default();
        if info.found.is_none() {
            return Err(format!("link '{name}' is not resolved (nothing to embed): {}", info.issue.unwrap_or_default()));
        }
        self.links.remove(i);
        for m in &info.members {
            self.managed.remove(m);
            if let Some(sg) = self.subgraphs.get_mut(m) {
                sg.meta.remove("embedded");
            }
        }
        Ok(info.members.len())
    }

    /// Remove a link and the graphs it brought in. Returns how many nodes still refer to the removed name.
    pub fn remove_link(&mut self, name: &str) -> usize {
        self.links.retain(|l| l.name != name);
        if let Some(info) = self.link_info.remove(name) {
            for m in info.members {
                self.managed.remove(&m);
                self.subgraphs.remove(&m);
            }
        }
        self.nodes.values().filter(|n| n.kind == "subgraph" && n.strs.get("graph").map(|g| g == name).unwrap_or(false)).count()
    }

    /// Point a link at another file (and optionally another graph in it), keeping its name so instances follow.
    pub fn relink(&mut self, name: &str, path: &str, graph: Option<String>) -> bool {
        match self.links.iter_mut().find(|l| l.name == name) {
            Some(l) => {
                l.path = path.replace('\\', "/");
                l.graph = graph;
                true
            }
            None => false,
        }
    }

    /// Read a package file and resolve its live links. The strings are link problems (not fatal). The graph is not
    /// validated, so that an editor can open a package that is broken (see [`Graph::validate`]).
    pub fn load_file(path: &std::path::Path) -> Result<(Graph, Vec<String>), Error> {
        let text = read_disk_text(path).map_err(|e| Error::Parse(0, format!("{}: {e}", path.display())))?;
        let mut g = Graph::parse_unchecked(&text)?;
        g.migrate_outputs();
        let issues = g.resolve_links(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
        Ok((g, issues))
    }

    /// What nodes (and legacy outputs) outside `sel` read from it: each selected node and pin, once.
    fn read_outside(&self, sel: &BTreeSet<u32>) -> Vec<(u32, Option<String>)> {
        let mut outs: Vec<(u32, Option<String>)> = Vec::new();
        for (oid, on) in &self.nodes {
            if sel.contains(oid) {
                continue;
            }
            for (slot, src) in &on.ins {
                let out = (*src, pin_read(&self.nodes, *src, on.in_out.get(slot)));
                if sel.contains(src) && !outs.contains(&out) {
                    outs.push(out);
                }
            }
        }
        for (_, s) in &self.outputs {
            if sel.contains(s) && !outs.contains(&(*s, None)) {
                outs.push((*s, None));
            }
        }
        outs
    }

    /// Replace the selected nodes with a subgraph instance. Returns the new node id.
    pub fn extract_subgraph(&mut self, sel: &BTreeSet<u32>, name: &str) -> Result<u32, String> {
        let sel: BTreeSet<u32> = sel.iter().copied().filter(|i| self.nodes.contains_key(i)).collect();
        if sel.is_empty() {
            return Err("select at least one node".into());
        }
        if self.subgraphs.contains_key(name) {
            return Err(format!("subgraph '{name}' already exists"));
        }
        if sel.iter().any(|i| self.nodes.get(i).is_some_and(|n| n.kind == "output")) {
            return Err("output nodes cannot be placed inside a subgraph".into());
        }
        let mut sub = Graph { size: self.size, height: self.height, ..Default::default() };
        let mut map: HashMap<u32, u32> = HashMap::new();
        let mut next = 1u32;
        for id in &sel {
            map.insert(*id, next);
            next += 1;
        }
        // what the instance reads from outside: (node, pin) in the order of its inputs
        let mut ext: Vec<(u32, Option<String>)> = Vec::new();
        let mut input_ids: HashMap<(u32, Option<String>), u32> = HashMap::new();
        for id in &sel {
            let Some(n) = self.nodes.get(id) else { continue };
            let Some(new) = map.get(id).copied() else { continue };
            let mut nn = n.clone();
            nn.ins.clear();
            for (slot, src) in &n.ins {
                if let Some(m) = map.get(src) {
                    nn.ins.insert(slot.clone(), *m);
                } else {
                    let from = (*src, pin_read(&self.nodes, *src, n.in_out.get(slot)));
                    let iid = if let Some(x) = input_ids.get(&from) {
                        *x
                    } else {
                        if ext.len() >= 4 {
                            return Err("a subgraph supports at most 4 external inputs".into());
                        }
                        let idx = ext.len();
                        ext.push(from.clone());
                        let iid = next;
                        next += 1;
                        let mut inode = Node { kind: "input".into(), ..Default::default() };
                        inode.nums.insert("idx".into(), idx as f32);
                        inode.pos = [n.pos[0] - 150.0, n.pos[1]];
                        sub.nodes.insert(iid, inode);
                        input_ids.insert(from, iid);
                        iid
                    };
                    nn.ins.insert(slot.clone(), iid);
                    // (the pin is read outside now, by the instance)
                    nn.in_out.remove(slot);
                }
            }
            sub.nodes.insert(new, nn);
        }
        let outs = self.read_outside(&sel);
        if outs.len() > 1 {
            return Err(format!("the selection has {} outputs used elsewhere; a subgraph supports exactly one", outs.len()));
        }
        let out_pin = outs.first().and_then(|(_, pin)| pin.clone());
        let out_id = match outs.first() {
            Some((x, _)) => *x,
            None => {
                let consumed: BTreeSet<u32> = sel.iter().filter_map(|i| self.nodes.get(i)).flat_map(|n| n.ins.values().copied()).collect();
                *sel.iter().rev().find(|i| !consumed.contains(i)).unwrap_or_else(|| sel.iter().next_back().unwrap_or(&0))
            }
        };
        {
            let onode_id = sub.nodes.keys().max().copied().unwrap_or(0) + 1;
            let inner = map.get(&out_id).copied().unwrap_or(0);
            let opos = sub.nodes.get(&inner).map(|n| [n.pos[0] + 150.0, n.pos[1]]).unwrap_or([0.0, 0.0]);
            let mut o = Node { kind: "output".into(), pos: opos, ..Default::default() };
            o.strs.insert("usage".into(), "out".into());
            o.ins.insert("in".into(), inner);
            if let Some(pin) = out_pin {
                o.in_out.insert("in".into(), pin);
            }
            sub.nodes.insert(onode_id, o);
        }
        let (mut cx, mut cy) = (0.0, 0.0);
        for n in sel.iter().filter_map(|id| self.nodes.get(id)) {
            cx += n.pos[0];
            cy += n.pos[1];
        }
        let new_id = self.nodes.keys().max().copied().unwrap_or(0) + 1;
        for id in &sel {
            self.nodes.remove(id);
        }
        let mut inst = Node { kind: "subgraph".into(), pos: [cx / sel.len() as f32, cy / sel.len() as f32], ..Default::default() };
        inst.strs.insert("graph".into(), name.to_string());
        for (i, (e, pin)) in ext.iter().enumerate() {
            inst.ins.insert(format!("in{i}"), *e);
            if let Some(pin) = pin {
                inst.in_out.insert(format!("in{i}"), pin.clone());
            }
        }
        self.nodes.insert(new_id, inst);
        for n in self.nodes.values_mut() {
            for (slot, v) in n.ins.iter_mut() {
                if *v == out_id {
                    *v = new_id;
                    // (the instance has the one output)
                    n.in_out.remove(slot);
                }
            }
        }
        self.outputs.retain(|(_, s)| !sel.contains(s) || *s == out_id);
        for (_, s) in self.outputs.iter_mut() {
            if *s == out_id {
                *s = new_id;
            }
        }
        self.subgraphs.insert(name.to_string(), sub);
        Ok(new_id)
    }

    // ---------------- evaluation ----------------
    /// Every output of the graph at `size` (the graph's own size when `None`), in the default [`EvalContext`].
    pub fn evaluate(&self, size: Option<usize>) -> Result<BTreeMap<String, Tex>, Error> {
        self.evaluate_in(&EvalContext::default(), size, &[])
    }

    /// Like [`Self::evaluate`], with images for the graph's `input` nodes (`images[idx]`; missing ones read as
    /// black).
    pub fn evaluate_with_inputs(&self, size: Option<usize>, images: &[Option<Tex>]) -> Result<BTreeMap<String, Tex>, Error> {
        self.evaluate_in(&EvalContext::default(), size, images)
    }

    /// Like [`Self::evaluate_with_inputs`], in the host's context `ec` (cancel flag, logging, profiler, memory
    /// counter). A cancelled evaluation returns [`Error::Cancelled`].
    pub fn evaluate_in(&self, ec: &EvalContext, size: Option<usize>, images: &[Option<Tex>]) -> Result<BTreeMap<String, Tex>, Error> {
        crate::context::scoped(ec, || self.evaluate_impl(ec, size, images))
    }

    fn evaluate_impl(&self, ec: &EvalContext, size: Option<usize>, images: &[Option<Tex>]) -> Result<BTreeMap<String, Tex>, Error> {
        // the images held as node results are (grey ones in a quarter of the memory), one copy at a time
        let ext: Vec<Option<Val>> = images.iter().map(|t| t.as_ref().map(|t| Val::of(t.clone()))).collect();
        self.evaluate_vals(ec, size, ext, None)
    }

    /// Like [`Self::evaluate_in`], taking the images: none is copied (a grey one is held in a quarter of the memory).
    pub fn evaluate_owned(&self, ec: &EvalContext, size: Option<usize>, images: Vec<Option<Tex>>) -> Result<BTreeMap<String, Tex>, Error> {
        crate::context::scoped(ec, || self.evaluate_vals(ec, size, images.into_iter().map(|t| t.map(Val::of)).collect(), None))
    }

    fn evaluate_vals(
        &self,
        ec: &EvalContext,
        size: Option<usize>,
        ext: Vec<Option<Val>>,
        instance: Option<(&GraphInstance<'_>, &[&str])>,
    ) -> Result<BTreeMap<String, Tex>, Error> {
        if instance.is_none() {
            self.validate()?;
        }
        match self.evaluate_outputs(ec, size, &ext, true, instance) {
            Err(Error::GpuReadback(..)) if !ec.is_cancelled() => {
                if let Some(done) = &ec.nodes_done {
                    done.store(0, std::sync::atomic::Ordering::Relaxed);
                }
                self.evaluate_outputs(ec, size, &ext, false, instance)
            }
            result => result,
        }
    }

    fn evaluate_outputs(
        &self,
        ec: &EvalContext,
        size: Option<usize>,
        ext: &[Option<Val>],
        resident: bool,
        instance: Option<(&GraphInstance<'_>, &[&str])>,
    ) -> Result<BTreeMap<String, Tex>, Error> {
        let size = size.unwrap_or(self.size);
        if size == 0 || size > 8192 {
            return Err(Error::BadSize(size));
        }
        let bound = instance.map(|(instance, _)| instance);
        let gin =
            crate::func::graph_inputs_from(self.nodes.iter().map(|(id, node)| bound.and_then(|instance| instance.nodes.get(id)).unwrap_or(node)));
        let ctx = Ctx {
            resident,
            size,
            height: self.height_for(size),
            nominal: self.size,
            graph: None,
            lib: &self.subgraphs,
            res: &self.resources,
            so: bound.map_or(self.seed_offset, |instance| instance.seed),
            depth: 0,
            gin: &gin,
            instance: bound,
            plan: bound.map(|instance| instance.plan),
            ec,
        };
        let mut outs = self.output_nodes();
        if let Some((_, retained)) = instance {
            outs.retain(|(_, name, _)| retained.contains(&name.as_str()));
        }
        let keep: Vec<u32> = outs.iter().map(|o| o.2).collect();
        let mut cache = Cache::for_graph(self, &keep, false, &ec.live);
        let mut res = BTreeMap::new();
        for (_, name, id) in &outs {
            let t = eval_node(self, *id, &ctx, ext, &mut cache)?;
            res.insert(name.clone(), (*id, t));
        }
        if ec.logging.memory {
            let (now, peak) = ec.live.get();
            eprintln!(
                "  eval {size}: {} nodes, own peak {} MB, held at end {} MB, context now {} MB, context peak {} MB",
                self.nodes.len(),
                cache.peak >> 20,
                cache.live >> 20,
                now >> 20,
                peak >> 20
            );
        }
        // hand the textures over without copying when no other output shares them
        cache.map.clear();
        res.into_iter().map(|(k, (id, v))| Ok((k, v.into_host(id, ec)?.into_tex()))).collect()
    }

    /// The nodes an evaluation of the graph's outputs runs at its own level (what [`EvalContext::nodes_done`] counts
    /// up to; a node with several outputs read may count once per output).
    pub fn work_nodes(&self) -> usize {
        let outs: Vec<u32> = self.output_nodes().iter().map(|o| o.2).collect();
        reached(self, &outs).len()
    }

    /// Evaluate a single node (editor preview). Ignores unrelated incomplete nodes.
    pub fn evaluate_node(&self, id: u32, size: usize) -> Result<Tex, Error> {
        self.evaluate_node_in(&EvalContext::default(), id, size)
    }

    /// Like [`Self::evaluate_node`], in the host's context `ec`.
    pub fn evaluate_node_in(&self, ec: &EvalContext, id: u32, size: usize) -> Result<Tex, Error> {
        crate::context::scoped(ec, || self.evaluate_node_impl(ec, id, size))
    }

    fn evaluate_node_impl(&self, ec: &EvalContext, id: u32, size: usize) -> Result<Tex, Error> {
        if size == 0 || size > 8192 {
            return Err(Error::BadSize(size));
        }
        self.check_cycles()?;
        let gin = crate::func::graph_inputs(self);
        let ctx = Ctx {
            resident: false,
            size,
            height: self.height_for(size),
            nominal: self.size,
            graph: None,
            lib: &self.subgraphs,
            res: &self.resources,
            so: self.seed_offset,
            depth: 0,
            gin: &gin,
            instance: None,
            plan: None,
            ec,
        };
        let mut cache = Cache::for_graph(self, &[id], false, &ec.live);
        let t = eval_node(self, id, &ctx, &[], &mut cache)?;
        cache.map.clear();
        Ok(t.into_host(id, ec)?.into_tex())
    }

    /// Evaluate every node once (shared cache). Produces thumbnails, per-node errors and
    /// full-resolution textures for the ids in `keep`. Result is Send, so it can run on a worker thread.
    pub fn evaluate_all(&self, size: usize, keep: &[u32]) -> EvalAll {
        self.evaluate_all_in(&EvalContext::default(), size, keep)
    }

    /// Like [`Self::evaluate_all`], in the host's context `ec`: once `ec` is cancelled the run stops within a few rows
    /// and its (partial) result must be thrown away.
    pub fn evaluate_all_in(&self, ec: &EvalContext, size: usize, keep: &[u32]) -> EvalAll {
        crate::context::scoped(ec, || self.evaluate_all_impl(ec, size, keep, None, None, &mut |_| {}))
    }

    /// Editor evaluation in the context `ec`: reuses every node result whose settings and inputs are unchanged
    /// (`cache` outlives the call), evaluates `first` (the node shown in the 2D view) before anything else and hands it
    /// to `on_first` at once.
    pub fn evaluate_all_cached(
        &self,
        ec: &EvalContext,
        size: usize,
        keep: &[u32],
        cache: &Arc<NodeCache>,
        first: Option<u32>,
        on_first: &mut dyn FnMut(&Tex),
    ) -> EvalAll {
        crate::context::scoped(ec, || self.evaluate_all_impl(ec, size, keep, Some(cache.clone()), first, on_first))
    }

    fn evaluate_all_impl(
        &self,
        ec: &EvalContext,
        size: usize,
        keep: &[u32],
        persist: Option<Arc<NodeCache>>,
        first: Option<u32>,
        on_first: &mut dyn FnMut(&Tex),
    ) -> EvalAll {
        let mut r = EvalAll { thumbs: HashMap::new(), errors: HashMap::new(), kept: HashMap::new(), sizes: HashMap::new() };
        if size == 0 || size > 8192 {
            r.errors.insert(0, Error::BadSize(size).to_string());
            return r;
        }
        if let Err(e) = self.check_cycles() {
            r.errors.insert(0, e.to_string());
            return r;
        }
        let gin = crate::func::graph_inputs(self);
        let ctx = Ctx {
            resident: false,
            size,
            height: self.height_for(size),
            nominal: self.size,
            graph: None,
            lib: &self.subgraphs,
            res: &self.resources,
            so: self.seed_offset,
            depth: 0,
            gin: &gin,
            instance: None,
            plan: None,
            ec,
        };
        // thumbnails are taken the moment a node finishes, so intermediate textures can be freed as soon as every
        // node that reads them is done (a big graph at 4K would not fit in memory otherwise)
        let mut cache = Cache::for_graph(self, keep, true, &ec.live);
        if let Some(p) = persist {
            cache.hashes = hash_graph(self, &ctx);
            cache.persist = Some(p);
        }
        // the 2D-view node first, then the outputs (3D view), then everything else (thumbnails)
        let mut order: Vec<u32> = Vec::new();
        order.extend(first.filter(|f| self.nodes.contains_key(f)));
        order.extend(keep.iter().copied().filter(|k| self.nodes.contains_key(k)));
        order.extend(self.nodes.keys().copied());
        let mut seen = HashSet::new();
        order.retain(|i| seen.insert(*i));
        for id in order {
            match eval_node(self, id, &ctx, &[], &mut cache).and_then(|value| value.into_host(id, ec)) {
                Ok(t) => {
                    if Some(id) == first || keep.contains(&id) {
                        let t = t.tex();
                        if Some(id) == first {
                            on_first(&t);
                        }
                        if keep.contains(&id) {
                            r.kept.insert(id, t);
                        }
                    }
                }
                Err(Error::Cancelled) => break,
                Err(e) => {
                    r.errors.insert(id, e.to_string());
                }
            }
        }
        r.thumbs = cache.thumbs.take().unwrap_or_default();
        r.sizes = std::mem::take(&mut cache.sizes);
        if let Some(p) = &cache.persist {
            p.trim();
        }
        r
    }
}

/// Node results kept between editor evaluations, found by a hash of everything a result depends on.
/// Least recently used entries are dropped when the memory budget is exceeded.
pub struct NodeCache {
    inner: Mutex<NcInner>,
    budget: usize,
    hits: std::sync::atomic::AtomicUsize,
    misses: std::sync::atomic::AtomicUsize,
}

struct NcInner {
    map: HashMap<u64, NcEntry>,
    bytes: usize,
    tick: u64,
}

struct NcEntry {
    data: NcData,
    thumb: Vec<u8>,
    size: usize,
    stamp: u64,
}

/// Grayscale textures (most of a typical graph) are stored with one channel: a quarter of the memory.
enum NcData {
    Gray(usize, usize, Vec<f32>),
    Rgba(Arc<Tex>),
}

impl NcData {
    fn pack(t: &Arc<Tex>) -> NcData {
        if t.px.iter().all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 1.0) {
            NcData::Gray(t.w, t.h, t.px.iter().map(|p| p[0]).collect())
        } else {
            NcData::Rgba(t.clone())
        }
    }
    fn unpack(&self) -> Arc<Tex> {
        match self {
            NcData::Gray(w, h, v) => {
                let mut px = vec![[0.0f32, 0.0, 0.0, 1.0]; v.len()];
                crate::par::for_each_run_mut(&mut px, 1 << 16, |first, dst| {
                    for (d, s) in dst.iter_mut().zip(v.iter().skip(first)) {
                        *d = [*s, *s, *s, 1.0];
                    }
                });
                Arc::new(Tex { w: *w, h: *h, px })
            }
            NcData::Rgba(t) => t.clone(),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            NcData::Gray(_, _, v) => v.len() * 4,
            NcData::Rgba(t) => t.px.len() * 16,
        }
    }
}

impl NodeCache {
    pub fn new(budget_mb: usize) -> NodeCache {
        NodeCache {
            inner: Mutex::new(NcInner { map: HashMap::new(), bytes: 0, tick: 0 }),
            budget: budget_mb.max(16) << 20,
            hits: Default::default(),
            misses: Default::default(),
        }
    }

    /// The budget a caller with no better idea passes to [`NodeCache::new`], in MB.
    pub const DEFAULT_BUDGET_MB: usize = 3072;

    fn get(&self, h: u64) -> Option<(Arc<Tex>, Vec<u8>, usize)> {
        let mut g = self.inner.lock().ok()?;
        g.tick += 1;
        let tick = g.tick;
        use std::sync::atomic::Ordering::Relaxed;
        match g.map.get_mut(&h) {
            Some(e) => {
                e.stamp = tick;
                self.hits.fetch_add(1, Relaxed);
                Some((e.data.unpack(), e.thumb.clone(), e.size))
            }
            None => {
                self.misses.fetch_add(1, Relaxed);
                None
            }
        }
    }

    fn put(&self, h: u64, tex: Arc<Tex>, thumb: Vec<u8>, size: usize) {
        if let Ok(mut g) = self.inner.lock() {
            g.tick += 1;
            let tick = g.tick;
            let data = NcData::pack(&tex);
            let bytes = data.bytes();
            if let Some(old) = g.map.insert(h, NcEntry { data, thumb, size, stamp: tick }) {
                g.bytes -= old.data.bytes();
            }
            g.bytes += bytes;
        }
    }

    fn trim(&self) {
        if let Ok(mut g) = self.inner.lock() {
            while g.bytes > self.budget && g.map.len() > 1 {
                let oldest = g.map.iter().min_by_key(|(_, e)| e.stamp).map(|(k, _)| *k);
                match oldest {
                    Some(k) => {
                        if let Some(e) = g.map.remove(&k) {
                            g.bytes -= e.data.bytes();
                        }
                    }
                    None => break,
                }
            }
        }
    }

    /// (reused, recomputed) node counts since the cache was created.
    pub fn stats(&self) -> (usize, usize) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.hits.load(Relaxed), self.misses.load(Relaxed))
    }

    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.map.clear();
            g.bytes = 0;
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.map.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> usize {
        self.inner.lock().map(|g| g.bytes).unwrap_or(0)
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf29ce484222325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 = (self.0 ^ *x as u64).wrapping_mul(0x100000001b3);
        }
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.bytes(s.as_bytes());
    }
}

/// Folds the length and modification time of the files a node reads from disk (those the graph does not carry) into
/// `h`: a file can change under us, and what was drawn from it must then count as another result.
fn hash_files(n: &Node, ctx: &Ctx, h: &mut Fnv) {
    if matches!(n.kind.as_str(), "bitmap" | "svg" | "text") {
        for f in [n.strs.get("file"), n.strs.get("font")].into_iter().flatten() {
            if !ctx.res.contains_key(f)
                && let Some((len, modified)) = ctx.ec.files.stamp(f)
            {
                h.u64(len);
                h.u64(modified);
            }
        }
    }
}

/// Hash of every node (and named subgraph output) of the graph: kind, settings, evaluation context and the hashes of
/// its inputs. Two nodes with the same hash always produce the same texture.
fn hash_graph(g: &Graph, ctx: &Ctx) -> HashMap<Key, u64> {
    let mut env = Fnv::new();
    env.u64(g.default_format() as u64);
    // an accelerator can draw other pixels than the reference code: its tag is part of every result's identity
    if let Some(accelerator) = &ctx.ec.accelerator {
        env.str(accelerator.cache_tag());
    }
    for (k, sub) in ctx.lib {
        env.str(k);
        env.str(&sub.to_text());
        // (the files a subgraph reads are not in its text)
        for n in sub.nodes.values() {
            hash_files(n, ctx, &mut env);
        }
    }
    for (k, v) in ctx.res {
        env.str(k);
        env.u64(v.len() as u64);
        let mut f = Fnv::new();
        f.bytes(v);
        env.u64(f.0);
    }
    for (k, v) in ctx.gin {
        env.str(k);
        env.str(&v.show());
    }
    let mut memo: HashMap<Key, u64> = HashMap::new();
    // Hashes `root` and everything it reads, inputs first. The nodes still waiting for their inputs are kept in a list,
    // not on the call stack, so a chain of any length is hashed whole (the graph has no cycles: they are refused first).
    fn walk(g: &Graph, ctx: &Ctx, env: u64, root: u32, root_out: Option<&str>, memo: &mut HashMap<Key, u64>) {
        let mut todo: Vec<(u32, Option<String>)> = vec![(root, root_out.map(str::to_string))];
        while let Some((id, out)) = todo.last().cloned() {
            let out = out.as_deref();
            let key = cache_key(g, id, out);
            let Some(n) = g.nodes.get(&id).filter(|_| !memo.contains_key(&key)) else {
                todo.pop();
                continue;
            };
            let mut waiting = false;
            for (slot, rid) in &n.ins {
                let want = n.in_out.get(slot).map(String::as_str);
                if g.nodes.contains_key(rid) && !memo.contains_key(&cache_key(g, *rid, want)) {
                    todo.push((*rid, want.map(str::to_string)));
                    waiting = true;
                }
            }
            if waiting {
                continue;
            }
            let mut h = Fnv::new();
            h.u64(env);
            h.u64(ctx.size as u64);
            h.u64(ctx.height as u64);
            h.u64(ctx.nominal as u64);
            h.u64(ctx.so as u64);
            h.str(&n.kind);
            if key != Key::from(id) {
                h.str(out.unwrap_or(""));
            }
            for (k, v) in &n.nums {
                h.str(k);
                h.u64(v.to_bits() as u64);
            }
            for (k, v) in &n.strs {
                h.str(k);
                h.str(v);
            }
            hash_files(n, ctx, &mut h);
            for (slot, rid) in &n.ins {
                let want = n.in_out.get(slot).map(String::as_str);
                h.str(slot);
                h.str(want.unwrap_or(""));
                // (a link to a node the graph doesn't have counts as 0)
                h.u64(memo.get(&cache_key(g, *rid, want)).copied().unwrap_or(0));
            }
            memo.insert(key, h.0);
            todo.pop();
        }
    }
    for (id, n) in &g.nodes {
        walk(g, ctx, env.0, *id, None, &mut memo);
        for (slot, rid) in &n.ins {
            if let Some(w) = n.in_out.get(slot) {
                walk(g, ctx, env.0, *rid, Some(w), &mut memo);
            }
        }
    }
    memo
}

#[derive(Clone)]
enum NodeValue {
    Host(Val),
    Device(Arc<DeviceValue>),
    DeviceFloat(Arc<DeviceFloatValue>),
    DeviceColor(Arc<DeviceColorValue>),
}

struct DeviceValue {
    image: ResidentGrey16,
    host: OnceLock<Arc<crate::tex::Grey16>>,
}

struct DeviceFloatValue {
    image: ResidentGreyFloat,
    host: OnceLock<Arc<crate::tex::Grey>>,
}

struct DeviceColorValue {
    image: ResidentColor16,
    host: OnceLock<Val>,
}

impl NodeValue {
    fn dims(&self) -> (usize, usize) {
        match self {
            Self::Host(image) => image.dims(),
            Self::Device(value) => (value.image.w, value.image.h),
            Self::DeviceFloat(value) => (value.image.w, value.image.h),
            Self::DeviceColor(value) => (value.image.w, value.image.h),
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Host(image) => image.bytes(),
            Self::Device(value) => value.image.w * value.image.h * 2,
            Self::DeviceFloat(value) => value.image.w * value.image.h * 4,
            Self::DeviceColor(value) => value.image.w * value.image.h * 8,
        }
    }

    fn packed_grey(&self) -> Option<GreyInput<'_>> {
        match self {
            Self::Host(Val::Grey16(image)) => Some(GreyInput::Host(image)),
            Self::Device(value) => Some(GreyInput::Device(&value.image)),
            _ => None,
        }
    }

    fn color_input(&self) -> Option<ColorInput<'_>> {
        match self {
            Self::Host(Val::Rgba(image)) => Some(ColorInput::Host(image)),
            Self::DeviceColor(value) => Some(ColorInput::Device(&value.image)),
            _ => self.packed_grey().map(ColorInput::Grey),
        }
    }

    fn into_host(self, id: u32, ec: &EvalContext) -> Result<Val, Error> {
        let value = match self {
            Self::Host(image) => return Ok(image),
            Self::Device(value) => value,
            Self::DeviceFloat(value) => return value.materialize(id, ec),
            Self::DeviceColor(value) => return value.materialize(id, ec),
        };
        if let Some(image) = value.host.get() {
            return Ok(Val::Grey16(image.clone()));
        }
        let ResidentGrey16 { w, h, div, alpha, pixels } = &value.image;
        let read = pixels.read(&ec.cancel);
        if ec.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let px = read.map_err(|reason| Error::GpuReadback(id, reason))?;
        if Some(px.len()) != w.checked_mul(*h) {
            return Err(Error::GpuReadback(id, "incomplete grayscale result".into()));
        }
        if let Some(profiler) = &ec.profiler {
            profiler.resident_readback(id, px.len() * 2);
        }
        let image = Arc::new(crate::tex::Grey16 { w: *w, h: *h, div: *div, alpha: *alpha, px });
        Ok(Val::Grey16(value.host.get_or_init(|| image).clone()))
    }
}

impl DeviceFloatValue {
    fn materialize(&self, id: u32, ec: &EvalContext) -> Result<Val, Error> {
        if let Some(image) = self.host.get() {
            return Ok(Val::Grey(image.clone(), 1.0));
        }
        let ResidentGreyFloat { w, h, pixels } = &self.image;
        let read = pixels.read(&ec.cancel);
        if ec.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let px = read.map_err(|reason| Error::GpuReadback(id, reason))?;
        if Some(px.len()) != w.checked_mul(*h) {
            return Err(Error::GpuReadback(id, "incomplete float grayscale result".into()));
        }
        if let Some(profiler) = &ec.profiler {
            profiler.resident_readback(id, px.len() * 4);
        }
        let image = Arc::new(crate::tex::Grey { w: *w, h: *h, px });
        Ok(Val::Grey(self.host.get_or_init(|| image).clone(), 1.0))
    }
}

impl DeviceColorValue {
    fn materialize(&self, id: u32, ec: &EvalContext) -> Result<Val, Error> {
        if let Some(image) = self.host.get() {
            return Ok(image.clone());
        }
        let ResidentColor16 { w, h, pixels, .. } = &self.image;
        let read = pixels.read(&ec.cancel);
        if ec.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let pixels = read.map_err(|reason| Error::GpuReadback(id, reason))?;
        if Some(pixels.len()) != w.checked_mul(*h).and_then(|n| n.checked_mul(4)) {
            return Err(Error::GpuReadback(id, "incomplete color result".into()));
        }
        if let Some(profiler) = &ec.profiler {
            profiler.resident_readback(id, pixels.len() * 2);
        }
        let px = pixels.as_chunks::<4>().0.iter().map(|p| p.map(|channel| f32::from(channel) / 65535.0)).collect();
        let image = Val::of(Tex { w: *w, h: *h, px }).quantised(65535.0);
        Ok(self.host.get_or_init(|| image).clone())
    }
}

fn eval_node(g: &Graph, id: u32, ctx: &Ctx, ext: &[Option<Val>], cache: &mut Cache) -> Result<NodeValue, Error> {
    eval_node_out(g, id, None, ctx, ext, cache)
}

/// What the evaluation cache is keyed by: a plain node id (a node's default output), or for a named output the id with
/// a hash of the name above it. The whole id is kept, so no two nodes share a key.
type Key = u64;

/// Cache key for one named output of a graph node (plain node ids keep their own key).
fn out_key(id: u32, out: &str) -> Key {
    let mut h: u32 = 2166136261;
    for b in out.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    // (never zero in the high half, which only a plain id has)
    (Key::from(h | 1) << 32) | Key::from(id)
}

/// A profile span of `name` when profiling is on (see `crate::profile`); nothing otherwise.
fn prof(name: &str) -> Option<crate::profile::Span> {
    crate::profile::enabled().then(|| crate::profile::span(name))
}

/// Output size of a node, the Output Size setting: "Relative to input" (default; falls back to the parent size when
/// nothing is connected), "Relative to parent", or "Absolute" (a power of two). The relative modifier moves the size
/// up or down in powers of two (-12 .. +12), so a heavy node can run at half or quarter resolution.
fn node_size(n: &Node, ins: &BTreeMap<String, NodeValue>, ctx: &Ctx) -> (usize, usize) {
    let get = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let mode = get("osize_mode", 0.0).round() as i32;
    let rel = get("osize_rel", 0.0).round().clamp(-12.0, 12.0) as i32;
    // an exact size, usually computed from the graph's: log2 per axis (decompiled graphs size nodes this way)
    if let Some(ex) = n.nums.get("osize_log2_x") {
        let axis = |e: f32| 1usize << (e.round().clamp(0.0, 13.0) as u32);
        return (axis(*ex), axis(get("osize_log2_y", *ex)));
    }
    let (bw, bh) = match mode {
        2 => {
            let e = get("osize_abs", 9.0).round().clamp(4.0, 13.0) as u32;
            let w = (((1usize << e) as f64 * ctx.size as f64 / ctx.nominal.max(1) as f64).round() as usize).max(1);
            // the height keeps the graph's aspect ratio unless the node sets its own height exponent
            let h = match n.nums.get("osize_abs_h") {
                Some(eh) => {
                    (((1usize << (eh.round().clamp(4.0, 13.0) as u32)) as f64 * ctx.size as f64 / ctx.nominal.max(1) as f64).round() as usize).max(1)
                }
                None => ((w as f64 * ctx.height as f64 / ctx.size.max(1) as f64).round() as usize).max(1),
            };
            (w, h)
        }
        1 => (ctx.size, ctx.height),
        _ => ins.get("in").or_else(|| ins.get("bg")).or_else(|| ins.values().next()).map(NodeValue::dims).unwrap_or((ctx.size, ctx.height)),
    };
    let shift = |b: usize| -> usize {
        if mode != 2 && rel != 0 {
            let v = if rel > 0 { b << rel } else { (b >> (-rel)).max(1) };
            v.clamp(16.min(b), 8192)
        } else {
            b
        }
    };
    (shift(bw).clamp(1, 8192), shift(bh).clamp(1, 8192))
}

/// A node on its way: its inputs are evaluated one at a time ([`eval_node_out`]) before it is computed itself.
struct Pending<'g> {
    id: u32,
    key: Key,
    /// the named output asked for (`None` for a node's plain output)
    out: Option<String>,
    /// the persistent cache's hash of the result
    phash: Option<u64>,
    /// the node with its function-driven settings bound
    node: std::borrow::Cow<'g, Node>,
    /// its input links in slot order: slot, source node, the source's output read
    links: Vec<(String, u32, Option<String>)>,
    /// how many of `links` have been asked for
    next: usize,
    ins: BTreeMap<String, NodeValue>,
}

/// What looking at a node first gives: its result when it is cached, else the node to evaluate.
enum Start<'g> {
    Done(NodeValue),
    Todo(Box<Pending<'g>>),
}

/// The result of node `id` (its output `out`), evaluating whatever it reads first and keeping it in `cache`.
///
/// The nodes still waiting for inputs are kept in a list of their own, not on the call stack, so a long chain of
/// nodes needs no more stack than a short one. They are visited in the order a depth-first walk takes (inputs in slot
/// order, each before the node reading it), so what is computed, freed and reported is the same as by recursion.
fn eval_node_out(g: &Graph, id: u32, out: Option<&str>, ctx: &Ctx, ext: &[Option<Val>], cache: &mut Cache) -> Result<NodeValue, Error> {
    let mut stack: Vec<Box<Pending>> = match start_node(g, id, out, ctx, cache)? {
        Start::Done(t) => return Ok(t),
        Start::Todo(p) => vec![p],
    };
    loop {
        let Some(top) = stack.last_mut() else { return Err(Error::MissingNode(0, id)) };
        if let Some((slot, rid, want)) = top.links.get(top.next).cloned() {
            top.next += 1;
            match start_node(g, rid, want.as_deref(), ctx, cache)? {
                Start::Done(t) => {
                    top.ins.insert(slot, t);
                }
                // (the graph has no cycles, so a node is never in the list twice: the list is never longer than the graph)
                Start::Todo(p) => stack.push(p),
            }
            continue;
        }
        let Some(done) = stack.pop() else { return Err(Error::MissingNode(0, id)) };
        let t = finish_node(g, *done, ctx, ext, cache)?;
        match stack.last_mut() {
            None => return Ok(t),
            Some(parent) => {
                // the link just asked for
                if let Some((slot, _, _)) = parent.next.checked_sub(1).and_then(|i| parent.links.get(i)) {
                    parent.ins.insert(slot.clone(), t);
                }
            }
        }
    }
}

/// A file of the machine's file system, for the package functions that read files by path (embedding bitmaps on
/// [`Graph::publish`], live links, [`Graph::load_file`]). The web build has no file system: there every read fails
/// with an error. Evaluation itself reads files only through [`crate::context::FileSource`].
fn read_disk(path: &std::path::Path) -> Result<Vec<u8>, String> {
    #[cfg(not(target_family = "wasm"))]
    return std::fs::read(path).map_err(|e| e.to_string());
    #[cfg(target_family = "wasm")]
    return Err(format!("{}: no file system in the web build", path.display()));
}

/// [`read_disk`] as UTF-8 text.
fn read_disk_text(path: &std::path::Path) -> Result<String, String> {
    String::from_utf8(read_disk(path)?).map_err(|e| e.to_string())
}

/// The clock of [`crate::context::Logging::node_times`]. The web has no clock the engine can read: there the line
/// carries no time.
struct NodeClock(#[cfg(not(target_family = "wasm"))] std::time::Instant);

impl NodeClock {
    fn start() -> NodeClock {
        NodeClock(
            #[cfg(not(target_family = "wasm"))]
            std::time::Instant::now(),
        )
    }

    fn report(&self, id: u32, kind: &str) {
        #[cfg(not(target_family = "wasm"))]
        eprintln!("  {:>5.0} ms  node {id} {kind}", self.0.elapsed().as_secs_f64() * 1000.0);
        #[cfg(target_family = "wasm")]
        eprintln!("  node {id} {kind}");
    }
}

/// The first look at a node: its cached result, or the node with its settings bound, ready for its inputs.
fn start_node<'g>(g: &'g Graph, id: u32, out: Option<&str>, ctx: &Ctx<'g>, cache: &mut Cache) -> Result<Start<'g>, Error> {
    let key = cache_key(g, id, out);
    let out = if key != Key::from(id) { out } else { None };
    if let Some(t) = cache.map.get(&key) {
        return Ok(Start::Done(t.clone()));
    }
    if ctx.ec.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let phash = cache.hashes.get(&key).copied();
    if let (Some(p), Some(h)) = (&cache.persist, phash)
        && let Some((t, th, sz)) = p.get(h)
    {
        let t = NodeValue::Host(Val::Rgba(t));
        cache.put(key, t.clone());
        if key == Key::from(id) {
            cache.sizes.insert(id, sz);
            if let Some(tm) = cache.thumbs.as_mut() {
                tm.insert(id, th);
            }
        }
        return Ok(Start::Done(t));
    }
    let n = ctx.instance.and_then(|instance| instance.nodes.get(&id)).or_else(|| g.nodes.get(&id)).ok_or(Error::MissingNode(0, id))?;
    let node = if n.strs.keys().any(|k| k.starts_with("fn.")) {
        let _span = prof("node.bind_functions");
        std::borrow::Cow::Owned(bind_functions(id, n, ctx)?)
    } else {
        std::borrow::Cow::Borrowed(n)
    };
    let links = node.ins.iter().map(|(slot, rid)| (slot.clone(), *rid, node.in_out.get(slot).cloned())).collect();
    Ok(Start::Todo(Box::new(Pending { id, key, out: out.map(str::to_string), phash, node, links, next: 0, ins: BTreeMap::new() })))
}

/// Computes a node whose inputs are all in, stores the result and frees the inputs nobody else waits for.
fn finish_node(g: &Graph, p: Pending, ctx: &Ctx, ext: &[Option<Val>], cache: &mut Cache) -> Result<NodeValue, Error> {
    let Pending { id, key, out, phash, node, ins, .. } = p;
    let n: &Node = &node;
    let out = out.as_deref();
    // the node's own output size; inputs of another size are resampled to it
    let sz = node_size(n, &ins, ctx);
    let _node = ctx.ec.profiler.as_ref().and_then(|p| p.node_variant(id, n, sz));
    // (a compiled graph: enlarging keeps the edge instead of wrapping, and an operation that reads its input at positions it
    // works out itself reads the input as it is)
    let compiled = g.meta.contains_key("compiled");
    let reads_positions =
        compiled && matches!(n.kind.as_str(), "pixel_processor" | "transform_matrix" | "laplacian_sharpen" | "grad_warp" | "pattern_program");
    let resize_inputs = !reads_positions && ins.values().any(|t| t.dims() != sz);
    // Directional warp can sample a differently sized map at the output pixel centers,
    // giving the same values as resampling the whole map first. Its source still needs
    // to match the output size, and the host fallback still resizes both inputs.
    let samples_warp_map = n.kind == "dir_warp" && ins.iter().all(|(slot, image)| slot == "map" || image.dims() == sz);
    let node_ctx;
    let ctx = if sz.0 != ctx.size || sz.1 != ctx.height {
        node_ctx = Ctx {
            resident: ctx.resident,
            size: sz.0,
            height: sz.1,
            nominal: ctx.nominal,
            graph: compiled.then_some([ctx.size, ctx.height]),
            lib: ctx.lib,
            res: ctx.res,
            so: ctx.so,
            depth: ctx.depth,
            gin: ctx.gin,
            instance: ctx.instance,
            plan: ctx.plan,
            ec: ctx.ec,
        };
        &node_ctx
    } else {
        ctx
    };
    // an explicit per-node Output Format (8 / 16 bit integer, 16 bit float, 5, 6, 7: 8 bit, 16 bit and 16 bit float as compiled graphs do them) quantises the
    // result; 32 bit float is untouched
    // (a node's own value 4 forces 32-bit float even when the graph rounds everything else)
    let fmt = node_format(g, n);
    let read_rounding = matches!(fmt, 5 | 6) && matches!(n.kind.as_str(), "grad_warp" | "dir_warp" | "transform_matrix");
    // a format of whole 8- or 16-bit steps: results held in them
    let div = match fmt {
        crate::measured::FORMAT_8_BIT | crate::measured::COMPILED_8_BIT => Some(crate::measured::STEPS_8 as f32),
        crate::measured::FORMAT_16_BIT | crate::measured::COMPILED_16_BIT => Some(crate::measured::STEPS_16 as f32),
        _ => None,
    };
    let format = NodeFormat { code: fmt, read_rounding, steps: div.map(|div| crate::tex::Steps { read_rounding, fmt, div }) };
    // (for the node log: every input grey)
    let grey_ins = !ins.is_empty() && ins.values().all(|v| !matches!(v, NodeValue::Host(Val::Rgba(_)) | NodeValue::DeviceColor(_)));
    let resident = if compiled && (!resize_inputs || samples_warp_map) {
        if let Some(held) = forward_switch(g, n, &ins, ctx, fmt).or_else(|| forward_identity(n, ins.get("in"), sz, fmt)) {
            if matches!(held, NodeValue::Device(_) | NodeValue::DeviceFloat(_) | NodeValue::DeviceColor(_))
                && let Some(profiler) = &ctx.ec.profiler
            {
                profiler.resident_node(id, &n.kind);
            }
            Some(held)
        } else if ctx.resident {
            compute_resident_image(id, n, &ins, ctx, &format)?
        } else {
            None
        }
    } else {
        None
    };
    let t = if let Some(image) = resident {
        image
    } else {
        let mut ins = ins.into_iter().map(|(slot, image)| Ok((slot, image.into_host(id, ctx.ec)?))).collect::<Result<BTreeMap<_, _>, Error>>()?;
        if resize_inputs {
            resize_node_inputs(&mut ins, sz, compiled);
        }
        NodeValue::Host(compute_host_node(id, n, ins, ctx, ext, out, format)?)
    };
    if ctx.ec.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let _span = prof("node.store_and_free");
    if ctx.depth == 0
        && let Some(done) = &ctx.ec.nodes_done
    {
        done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    // Results that outlive the evaluation are kept as RGBA in the editor.
    let full =
        if cache.persist.is_some() || (key == Key::from(id) && cache.thumbs.is_some()) { Some(t.clone().into_host(id, ctx.ec)?.tex()) } else { None };
    let t = match (&full, &cache.persist) {
        (Some(f), Some(_)) => NodeValue::Host(Val::Rgba(f.clone())),
        _ => t,
    };
    cache.put(key, t.clone());
    if ctx.ec.logging.node_times {
        let form = match &t {
            NodeValue::Host(Val::Rgba(_)) => "rgba",
            NodeValue::Host(Val::Grey(..)) => "grey",
            NodeValue::Host(Val::Grey16(_)) => "grey in steps",
            NodeValue::Device(_) | NodeValue::DeviceFloat(_) => "grey in GPU memory",
            NodeValue::DeviceColor(_) => "color in GPU memory",
        };
        eprintln!(
            "         node {id} held {form} {}x{}{}; results held {} MB",
            sz.0,
            sz.1,
            if grey_ins { ", grey inputs" } else { "" },
            cache.live >> 20
        );
    }
    if key == Key::from(id) {
        cache.sizes.insert(id, sz.0);
    }
    let thumb = match &full {
        Some(f) if key == Key::from(id) => thumbnail(f),
        _ => Vec::new(),
    };
    if let (Some(p), Some(h), Some(f)) = (&cache.persist, phash, &full) {
        p.put(h, f.clone(), thumb.clone(), sz.0);
    }
    if key == Key::from(id)
        && let Some(th) = cache.thumbs.as_mut()
    {
        th.insert(id, thumb);
    }
    release_node_inputs(g, n, cache);
    Ok(t)
}

fn resize_node_inputs(ins: &mut BTreeMap<String, Val>, sz: (usize, usize), compiled: bool) {
    let _span = prof("node.resize_inputs");
    for t in ins.values_mut() {
        if t.dims() != sz {
            let full = t.tex();
            *t = Val::of(if compiled { ops::resize_keep_edge(&full, sz.0, sz.1) } else { ops::resize_to(&full, sz.0, sz.1) });
        }
    }
}

fn release_node_inputs(g: &Graph, n: &Node, cache: &mut Cache) {
    // Free inputs nobody else is waiting for.
    for (slot, rid) in &n.ins {
        let k = cache_key(g, *rid, n.in_out.get(slot).map(|s| s.as_str()));
        if let Some(c) = cache.left.get_mut(&k) {
            *c = c.saturating_sub(1);
            if *c == 0 && !cache.keep.contains(&k) {
                cache.drop_entry(k);
            }
        }
    }
}

struct NodeFormat {
    code: u32,
    read_rounding: bool,
    steps: Option<crate::tex::Steps>,
}

fn compute_host_node(
    id: u32,
    n: &Node,
    ins: BTreeMap<String, Val>,
    ctx: &Ctx,
    ext: &[Option<Val>],
    output: Option<&str>,
    NodeFormat { code: format, read_rounding, steps }: NodeFormat,
) -> Result<Val, Error> {
    // A secondary output runs the same node with its output index set.
    let extra = output.and_then(|name| node_outputs(&n.kind).iter().position(|&slot| slot == name)).filter(|&index| index > 0);
    let t = match n.kind.as_str() {
        "input" => {
            let idx = n.nums.get("idx").copied().unwrap_or(0.0).max(0.0) as usize;
            let _span = prof("node.input");
            let gray = n.nums.get("gray").copied().unwrap_or(0.0) > 0.5;
            match ext.get(idx).cloned().flatten() {
                // the luminance of a grey pixel, as the RGBA form computes it
                Some(t) if gray => match t.grey_in() {
                    Some(g) => Computed::Grey(g.floats().map(|v| lum([v, v, v, 1.0])), 1.0),
                    None => Computed::Rgba(ops::to_gray(&t.tex())),
                },
                Some(t) => Computed::Held(t),
                None => Computed::Rgba(Tex::solid(ctx.size, ctx.height, [0.0, 0.0, 0.0, 1.0])),
            }
        }
        "subgraph" => Computed::Held(run_sub(id, n, &ins, ctx, output)?),
        _ => {
            let t0 = ctx.ec.logging.node_times.then(NodeClock::start);
            let _span = crate::profile::enabled().then(|| crate::profile::span(&format!("node.{}", n.kind)));
            // grey inputs, an operation that keeps grey grey: computed on one channel
            let accelerated = match n.kind.as_str() {
                "pattern_program" => compute_gpu_pattern(id, n, &ins, ctx)?,
                "transform_matrix" | "blend" => compute_gpu_grey_pass(id, n, &ins, ctx, steps.as_ref())?,
                _ => None,
            };
            let grey = if accelerated.is_none() && extra.is_none() && GREY_KINDS.contains(&n.kind.as_str()) {
                compute_grey(id, n, &ins, ctx, steps.as_ref())?
            } else {
                None
            };
            let r = match accelerated.or(grey) {
                Some(c) => {
                    #[cfg(test)]
                    if matches!(c, Computed::Grey(..)) {
                        grey_tests::GREY_RUNS.with(|n| n.set(n.get() + 1));
                    }
                    c
                }
                None => {
                    let ins = expanded(&ins);
                    Computed::Rgba(match extra {
                        Some(i) => {
                            let mut n2 = n.clone();
                            n2.nums.insert("_out".into(), i as f32);
                            compute(id, &n2, &ins, ctx)?
                        }
                        None => compute(id, n, &ins, ctx)?,
                    })
                }
            };
            if let Some(t0) = t0 {
                t0.report(id, &n.kind);
            }
            r
        }
    };
    drop(ins);
    // a superseded evaluation can leave a half-finished texture behind: never keep or show it
    if ctx.ec.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let sz = (ctx.size, ctx.height);
    let t = if t.dims() != sz {
        let _span = prof("node.resize_result");
        Computed::Rgba(ops::resize_to(&t.into_tex(), sz.0, sz.1))
    } else {
        t
    };
    let t = match t {
        // (computed in its format's steps already)
        Computed::Stepped(v) => Val::Grey16(Rc::new(v)),
        // (a held result the node doesn't change is not copied)
        Computed::Held(v) if format == 0 && !read_rounding => v,
        Computed::Held(Val::Rgba(t)) => {
            let mut t = Rc::try_unwrap(t).unwrap_or_else(|rc| (*rc).clone());
            finish_format(&mut t, format, read_rounding);
            Val::of(t)
        }
        Computed::Held(v) => {
            // grey (in steps or not): its values
            let (mut t, a) = match v.grey_in() {
                Some(g) => (g.floats().into_owned(), g.alpha()),
                None => (Grey::solid(1, 1, 0.0), 1.0),
            };
            finish_format(&mut t, format, read_rounding);
            Val::Grey(Rc::new(t), a)
        }
        Computed::Rgba(mut t) => {
            finish_format(&mut t, format, read_rounding);
            // held grey when it came out grey
            Val::of(t)
        }
        Computed::Grey(mut t, a) => {
            finish_format(&mut t, format, read_rounding);
            Val::Grey(Rc::new(t), a)
        }
    };
    Ok(match steps {
        Some(steps) => t.quantised(steps.div),
        None => t,
    })
}

/// A node's result as computed, before its format and before it is held.
enum Computed {
    Rgba(Tex),
    /// grey values and their alpha (1 or +0)
    Grey(Grey, f32),
    /// a result held already (a graph input, a subgraph's output): taken over as it is unless the node changes it
    Held(Val),
    /// grey in the steps of the node's format, the format applied
    Stepped(crate::tex::Grey16),
}

impl Computed {
    fn dims(&self) -> (usize, usize) {
        match self {
            Computed::Rgba(t) => (t.w, t.h),
            Computed::Grey(t, _) => (t.w, t.h),
            Computed::Held(v) => v.dims(),
            Computed::Stepped(t) => (t.w, t.h),
        }
    }

    fn into_tex(self) -> Tex {
        match self {
            Computed::Rgba(t) => t,
            Computed::Grey(t, a) => t.expand(a),
            Computed::Held(v) => v.into_tex(),
            Computed::Stepped(t) => t.expand(),
        }
    }
}

/// The node's format applied to its result: for warps of an integer format a read rounded to 16 bits first.
fn finish_format<P: crate::tex::Pixel>(t: &mut crate::tex::Image<P>, fmt: u32, read_rounding: bool) {
    // a read of an integer image rounds to a 16-bit step, half up: what one read gives doesn't meet the node's own rounding (measured on warps)
    if read_rounding {
        let _span = prof("node.read_rounding");
        ops::round_to_16bit_read(t);
    }
    if fmt != 0 {
        let _span = prof("node.format");
        ops::apply_format(t, fmt);
    }
}

/// The inputs as RGBA images, for the operations that compute in RGBA (a grey input expanded: a copy).
fn expanded(ins: &BTreeMap<String, Val>) -> BTreeMap<String, Rc<Tex>> {
    ins.iter().map(|(k, v)| (k.clone(), v.tex())).collect()
}

/// Textures of finished nodes, plus what is needed to free them early.
struct Cache {
    map: HashMap<Key, NodeValue>,
    /// how many not-yet-evaluated links still read each cached entry; an entry is freed when it reaches zero
    left: HashMap<Key, u32>,
    keep: HashSet<Key>,
    /// thumbnails taken while evaluating (editor preview)
    thumbs: Option<HashMap<u32, Vec<u8>>>,
    sizes: HashMap<u32, usize>,
    /// bytes of the results in `map` (as held: 4-channel or grey float), and the most at once
    live: usize,
    peak: usize,
    /// where the evaluation's context counts the bytes held
    counter: LiveBytes,
    /// results that outlive one evaluation (editor): a node whose settings and inputs did not change is not recomputed
    persist: Option<Arc<NodeCache>>,
    hashes: HashMap<Key, u64>,
}

impl Cache {
    fn plain(counter: &LiveBytes) -> Cache {
        Cache {
            map: HashMap::new(),
            left: HashMap::new(),
            keep: HashSet::new(),
            thumbs: None,
            sizes: HashMap::new(),
            live: 0,
            peak: 0,
            counter: counter.clone(),
            persist: None,
            hashes: HashMap::new(),
        }
    }

    fn for_graph(g: &Graph, keep: &[u32], thumbs: bool, counter: &LiveBytes) -> Cache {
        let mut c = Cache::plain(counter);
        // only the links of nodes this evaluation runs count: a node that is never run never takes its inputs' counts
        // down, and would hold them to the end (an output not asked for, a subgraph's other outputs). Evaluating every
        // node (`thumbs`) runs them all.
        let runs = (!thumbs).then(|| reached(g, keep));
        for (id, n) in &g.nodes {
            if runs.as_ref().is_some_and(|r| !r.contains(id)) {
                continue;
            }
            for (slot, rid) in &n.ins {
                *c.left.entry(cache_key(g, *rid, n.in_out.get(slot).map(|s| s.as_str()))).or_insert(0) += 1;
            }
        }
        c.keep = keep.iter().map(|k| Key::from(*k)).collect();
        if thumbs {
            c.thumbs = Some(HashMap::new());
        }
        c
    }

    fn put(&mut self, key: Key, t: NodeValue) {
        let b = t.bytes();
        if let Some(old) = self.map.insert(key, t) {
            self.live = self.live.saturating_sub(old.bytes());
            self.counter.sub(old.bytes());
        }
        self.live += b;
        self.peak = self.peak.max(self.live);
        self.counter.add(b);
    }

    fn drop_entry(&mut self, key: Key) {
        if let Some(old) = self.map.remove(&key) {
            self.live = self.live.saturating_sub(old.bytes());
            self.counter.sub(old.bytes());
        }
    }
}

/// The nodes evaluating `from` runs: those and everything they read, through every input.
pub(crate) fn reached(g: &Graph, from: &[u32]) -> HashSet<u32> {
    let mut seen: HashSet<u32> = HashSet::new();
    let mut todo: Vec<u32> = from.to_vec();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(n) = g.nodes.get(&id) {
            todo.extend(n.ins.values().copied());
        }
    }
    seen
}

impl Drop for Cache {
    fn drop(&mut self) {
        self.counter.sub(self.live);
    }
}

/// Named outputs of a built-in node (empty = one plain output). The first name is the default output: a link that
/// names no output, or names the first one, reads it. Extra outputs are computed on demand by running the node's
/// own code with a hidden `_out` index (see `eval_node_out`), and cached separately.
pub fn node_outputs(kind: &str) -> &'static [&'static str] {
    match kind {
        "color_match" => &["output", "mask"],
        "quantize_color" => &["output", "id", "palette"],
        "weave_generator" => &["height", "mask_1", "mask_2", "opacity"],
        "rgba_split" => &["r", "g", "b", "a"],
        "alpha_split" => &["rgb", "alpha"],
        "voronoi" => &["output", "cell_id", "edge", "distance"],
        "cells" => &["output", "cell_id", "distance", "edge"],
        "splatter" => &["output", "id", "center", "size", "luminance"],
        "flood_fill_to_position" => &["position", "x", "y"],
        "flood_fill_to_bbox_size" => &["size", "width", "height"],
        _ => &[],
    }
}

/// What travels through a pin. Grayscale pins carry one value per pixel, colour pins carry RGB(A). Some tools keep
/// the two kinds apart; here every texture is stored as RGBA, so the type is what the editor shows and what
/// `output_pin` / `pin_types` report (a grayscale value fed to a colour input simply becomes a gray colour).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    Gray,
    Color,
    /// value pins (carried by function graphs and value nodes, not by textures)
    Float,
    Float2,
    Float3,
    Float4,
    Int,
    Int2,
    Int3,
    Int4,
    Bool,
    Text,
}

impl Pin {
    /// Carries a texture (grayscale or colour).
    pub fn is_image(self) -> bool {
        matches!(self, Pin::Gray | Pin::Color)
    }

    /// Number of components of a numeric value pin (1 for scalars, 0 for images, bool and text).
    pub fn lanes(self) -> usize {
        match self {
            Pin::Float | Pin::Int => 1,
            Pin::Float2 | Pin::Int2 => 2,
            Pin::Float3 | Pin::Int3 => 3,
            Pin::Float4 | Pin::Int4 => 4,
            _ => 0,
        }
    }

    /// Can an output of type `from` feed an input of this type without an explicit cast?
    /// Images convert freely between gray and colour; numeric values widen from integer to float and from bool to
    /// integer or float with the same number of components; nothing else converts implicitly (a float needs a cast
    /// to become an integer, an image never turns into a value or back).
    pub fn accepts(self, from: Pin) -> bool {
        if self == from {
            return true;
        }
        if self.is_image() || from.is_image() {
            return self.is_image() && from.is_image();
        }
        matches!(
            (self, from),
            (Pin::Float, Pin::Int)
                | (Pin::Float2, Pin::Int2)
                | (Pin::Float3, Pin::Int3)
                | (Pin::Float4, Pin::Int4)
                | (Pin::Int, Pin::Bool)
                | (Pin::Float, Pin::Bool)
        )
    }

    /// Short name for tooltips and file messages.
    pub fn name(self) -> &'static str {
        match self {
            Pin::Gray => "grayscale",
            Pin::Color => "color",
            Pin::Float => "float",
            Pin::Float2 => "float2",
            Pin::Float3 => "float3",
            Pin::Float4 => "float4",
            Pin::Int => "integer",
            Pin::Int2 => "integer2",
            Pin::Int3 => "integer3",
            Pin::Int4 => "integer4",
            Pin::Bool => "boolean",
            Pin::Text => "string",
        }
    }
}

/// What a built-in node's input slot expects when it is not an image slot. Image slots (the default) take any
/// texture. Value-typed slots belong to the function-graph and value nodes.
pub fn input_value_type(kind: &str, slot: &str) -> Option<Pin> {
    let _ = (kind, slot);
    None
}

/// Connections whose types do not fit: `(node, slot, message)`. The editor refuses these when a link is dragged;
/// a file that contains one is still loadable, the node simply reports the error when it is evaluated.
pub fn connection_issues(g: &Graph) -> Vec<(u32, String, String)> {
    let pins = pin_types(g);
    let mut issues = Vec::new();
    for (id, n) in &g.nodes {
        for (slot, src) in &n.ins {
            let Some(sn) = g.nodes.get(src) else { continue };
            let out = n.in_out.get(slot).map(|s| s.as_str());
            let from = if !node_outputs(&sn.kind).is_empty() && out.is_some() {
                let upstream = sn.ins.values().any(|s| pins.get(s) == Some(&Pin::Color));
                output_pin(&sn.kind, out, upstream, sn)
            } else {
                pins.get(src).copied().unwrap_or(Pin::Gray)
            };
            let want = input_value_type(&n.kind, slot).unwrap_or(Pin::Gray);
            if !want.accepts(from) {
                issues.push((*id, slot.clone(), format!("{} input '{slot}' needs {} but node {src} gives {}", n.kind, want.name(), from.name())));
            }
        }
    }
    issues
}

/// Output type of a built-in node. `any_color_in` says whether any connected input is a colour pin, `n` supplies
/// mode switches such as the Uniform Color node's grayscale flag.
pub fn output_pin(kind: &str, out: Option<&str>, any_color_in: bool, n: &Node) -> Pin {
    let flag = |k: &str| n.nums.get(k).copied().unwrap_or(0.0) > 0.5;
    match kind {
        // always colour
        "colorize" | "flood_fill_color" => Pin::Color,
        "tint"
        | "hsl"
        | "channel_mixer"
        | "merge"
        | "replace_color"
        | "chrominance_extract"
        | "normal"
        | "normal_blend"
        | "normal_combine"
        | "normal_invert"
        | "normal_normalize"
        | "normal_vector_rotation"
        | "alpha_merge"
        | "vector_warp_color" => Pin::Color,
        "color" | "bitmap" => {
            if flag("gray") {
                Pin::Gray
            } else {
                Pin::Color
            }
        }
        "color_match" => match out {
            Some("mask") => Pin::Gray,
            _ => Pin::Color,
        },
        "quantize_color" => match out {
            Some("id") => Pin::Gray,
            _ => Pin::Color,
        },
        "rgba_split" => Pin::Gray,
        "splatter" => match out {
            Some("center") | Some("size") => Pin::Color,
            Some("output") | None => {
                if any_color_in {
                    Pin::Color
                } else {
                    Pin::Gray
                }
            }
            _ => Pin::Gray,
        },
        "flood_fill_to_index" => Pin::Gray,
        "flood_fill_to_position" | "flood_fill_to_bbox_size" => match out {
            Some("x") | Some("y") | Some("width") | Some("height") => Pin::Gray,
            _ => Pin::Color,
        },
        "alpha_split" => match out {
            Some("alpha") => Pin::Gray,
            _ => Pin::Color,
        },
        // always grayscale
        "to_gray"
        | "grayscale_conversion"
        | "normal_to_height"
        | "ao"
        | "curvature"
        | "curvature_smooth"
        | "distance"
        | "histogram_range"
        | "histogram_scan"
        | "histogram_shift"
        | "threshold"
        | "flood_fill"
        | "flood_fill_random"
        | "flood_fill_gradient"
        | "edge_detect"
        | "color_to_mask"
        | "quantize"
        | "sobel"
        | "edge_wear"
        | "dirt"
        | "dust"
        | "bevel"
        | "channel" => Pin::Gray,
        // generators with no colour of their own
        "white_noise" | "gaussian_noise" | "perlin" | "fbm" | "clouds" | "voronoi" | "voronoi_fractal" | "bnw_spots" | "crystal"
        | "directional_noise" | "grunge" | "plasma" | "gradient" | "gradient_axial" | "gradient_circular" | "gradient_reflected" | "circle"
        | "rect" | "polygon" | "shape" | "star" | "starburst" | "checker" | "bricks" | "stripes" | "weave" | "cells" | "fractal_sum" | "waveform"
        | "gaussian_spots" | "triangle_grid" | "grunge_map" | "sdf_shape" | "sdf_op" | "sdf_mask" | "sdf_form" => Pin::Gray,
        "weave_generator" => Pin::Gray,
        // pixel processor and the scatter family produce colour only when colour comes in
        "pixel_processor" | "value_processor" | "fx_map" => Pin::Color,
        // everything else (blur, levels, blend, transform, warp, ...) keeps the type of what goes in
        _ => {
            if any_color_in {
                Pin::Color
            } else {
                Pin::Gray
            }
        }
    }
}

/// Type of every node's default output in a graph (nodes in a cycle or with unknown inputs count as gray).
pub fn pin_types(g: &Graph) -> HashMap<u32, Pin> {
    fn go(g: &Graph, id: u32, memo: &mut HashMap<u32, Pin>, depth: u32) -> Pin {
        if let Some(p) = memo.get(&id) {
            return *p;
        }
        let Some(n) = g.nodes.get(&id) else { return Pin::Gray };
        if depth > 512 {
            return Pin::Gray;
        }
        memo.insert(id, Pin::Gray); // breaks cycles
        let mut any_color = false;
        for (slot, src) in &n.ins {
            let want = n.in_out.get(slot).map(|s| s.as_str());
            let sn = g.nodes.get(src);
            let p = match (sn, want) {
                // a named output of a built-in multi-output node has its own type
                (Some(sn), Some(o)) if !node_outputs(&sn.kind).is_empty() => {
                    let upstream = sn.ins.values().any(|s| go(g, *s, memo, depth + 1) == Pin::Color);
                    output_pin(&sn.kind, Some(o), upstream, sn)
                }
                _ => go(g, *src, memo, depth + 1),
            };
            if p == Pin::Color {
                any_color = true;
            }
        }
        let p = match n.kind.as_str() {
            "input" => {
                if n.nums.get("gray").copied().unwrap_or(0.0) > 0.5 {
                    Pin::Gray
                } else {
                    Pin::Color
                }
            }
            // a graph node is treated as colour (its outputs are defined by the graph inside)
            "subgraph" => Pin::Color,
            "output" | "dot" => {
                if any_color {
                    Pin::Color
                } else {
                    Pin::Gray
                }
            }
            k => output_pin(k, None, any_color, n),
        };
        memo.insert(id, p);
        p
    }
    let mut memo = HashMap::new();
    for id in g.nodes.keys() {
        go(g, *id, &mut memo, 0);
    }
    memo
}

/// Cache key of an output: the node id, or a hash of (id, output name) for a named output of a graph node or of a
/// built-in multi-output node (its default output keeps the plain node id).
fn cache_key(g: &Graph, id: u32, out: Option<&str>) -> Key {
    let Some(o) = out else { return Key::from(id) };
    match g.nodes.get(&id) {
        Some(n) if n.kind == "subgraph" => out_key(id, o),
        Some(n) => match node_outputs(&n.kind).iter().position(|x| *x == o) {
            Some(i) if i > 0 => out_key(id, o),
            _ => Key::from(id),
        },
        None => Key::from(id),
    }
}

fn thumbnail(t: &Tex) -> Vec<u8> {
    // a non-square texture is fitted inside the square thumbnail with dark bars instead of being stretched
    let (w, h) = (t.w.max(1) as f32, t.h.max(1) as f32);
    let k = (THUMB as f32 / w).min(THUMB as f32 / h);
    let (tw, th) = ((w * k).round().max(1.0), (h * k).round().max(1.0));
    let (ox, oy) = ((THUMB as f32 - tw) * 0.5, (THUMB as f32 - th) * 0.5);
    let mut px = Vec::with_capacity(THUMB * THUMB * 4);
    for y in 0..THUMB {
        for x in 0..THUMB {
            let (fx, fy) = (x as f32 + 0.5 - ox, y as f32 + 0.5 - oy);
            if fx < 0.0 || fy < 0.0 || fx >= tw || fy >= th {
                px.extend_from_slice(&[30, 30, 30, 255]);
                continue;
            }
            let p = t.sample(fx / tw, fy / th);
            for c in p.iter().take(3) {
                px.push((c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
            px.push(255);
        }
    }
    px
}

/// Function-driven parameters: `fn.<param>="name"` runs that function and stores the result in `<param>`;
/// `fa.<param>.<input>` numbers feed the function's inputs.
fn bind_functions(id: u32, n: &Node, ctx: &Ctx) -> Result<Node, Error> {
    use crate::func::FnEnv;
    let mut out = n.clone();
    // the size first: the other parameters can read it
    let is_size = |k: &str| k.starts_with("fn.osize_");
    let (sized, rest): (Vec<_>, Vec<_>) = n.strs.iter().partition(|(k, _)| is_size(k));
    for (k, fname) in sized.into_iter().chain(rest) {
        let Some(param) = k.strip_prefix("fn.") else { continue };
        if fname.is_empty() {
            continue;
        }
        let prog = ctx.program(fname).map_err(|e| Error::BadParam(id, format!("{k}: {e}")))?;
        let mut env = FnEnv::new([ctx.size as f64, ctx.height as f64]);
        env.seed = ctx.so;
        env.graph_inputs = ctx.gin;
        if !is_size(k)
            && let Some(x) = out.nums.get("osize_log2_x")
        {
            let y = out.nums.get("osize_log2_y").unwrap_or(x);
            let axis = |e: f32| f64::from(1u32 << (e.round().clamp(0.0, 13.0) as u32));
            env.own = Some([axis(*x), axis(*y)]);
        }
        let prefix = format!("fa.{param}.");
        for (ak, av) in &n.nums {
            if let Some(input) = ak.strip_prefix(&prefix) {
                let pin = prog.inputs.iter().find(|i| i.0 == input).map(|i| i.1).unwrap_or(crate::graph::Pin::Float);
                env.args.insert(input.to_string(), crate::func::coerce(&crate::func::Value::float(*av as f64), pin));
            }
        }
        let v = prog.run(&env, [0.5, 0.5]).map_err(|e| Error::BadParam(id, format!("{k}: {e}")))?;
        match v {
            crate::func::Value::Str(s) => {
                out.strs.insert(param.to_string(), s.to_string());
            }
            v => {
                out.nums.insert(param.to_string(), v.as_f32());
            }
        }
    }
    Ok(out)
}

/// Pixel processor / value processor driven by a function graph.
fn read_resource(id: u32, file: &str, ctx: &Ctx) -> Result<Vec<u8>, Error> {
    match ctx.res.get(file) {
        Some(b) => Ok(b.clone()),
        None => ctx.ec.files.read(file).map_err(|e| Error::BadParam(id, format!("cannot read '{file}': {e}"))),
    }
}

fn run_function_image(id: u32, n: &Node, fname: &str, ins: &BTreeMap<String, Rc<Tex>>, ctx: &Ctx, once: bool) -> Result<Tex, Error> {
    let images = IMAGE_SLOTS.iter().filter_map(|slot| ins.get(*slot).map(|t| (*slot, crate::tex::ImgRef::Rgba(t)))).collect();
    Ok(run_function(id, n, fname, images, ctx, once, None)?.into_tex())
}

fn pixel_function_env<'a>(n: &Node, ctx: &'a Ctx, prog: &crate::func::Prog) -> crate::func::FnEnv<'a> {
    let size = [ctx.size as f64, ctx.height as f64];
    let mut env = crate::func::FnEnv::new(size);
    // The built-in size is the node's. `$outputsize` refers to the graph.
    if let Some(graph) = ctx.graph {
        env.own = Some(size);
        env.size = graph.map(|side| side as f64);
    }
    env.seed = (n.nums.get("seed").copied().unwrap_or(0.0).max(0.0) as u32).wrapping_add(ctx.so);
    env.graph_inputs = ctx.gin;
    for (key, value) in &n.nums {
        if let Some(input) = key.strip_prefix("fa.") {
            let pin = prog.inputs.iter().find(|i| i.0 == input).map_or(Pin::Float, |i| i.1);
            env.args.insert(input.to_string(), crate::func::coerce(&crate::func::Value::float(f64::from(*value)), pin));
        }
    }
    env
}

/// A function run at every pixel ([`run_function_image`]) on the images `images` (by slot). `grey`: a function that gives one
/// number (or a boolean) at every pixel comes out as a grey image (the same values the RGBA form gives, `[v, v, v, 1]`).
/// `grey`: `Some` to give a grey result where it is one, computed in the steps given when it can be.
fn run_function(
    id: u32,
    n: &Node,
    fname: &str,
    images: Vec<(&str, crate::tex::ImgRef)>,
    ctx: &Ctx,
    once: bool,
    grey: Option<Option<&crate::tex::Steps>>,
) -> Result<Computed, Error> {
    use crate::func::Value;
    let prog = ctx.program(fname).map_err(|e| Error::BadParam(id, e))?;
    let (w, h) = (ctx.size, ctx.height);
    let mut env = pixel_function_env(n, ctx, &prog);
    env.images = images;
    if prog.uses_stats {
        env = env.with_stats();
    }
    let to_px = |v: &Value| -> [f32; 4] {
        match v {
            Value::Num(x) => match x.lanes {
                1 => [x.v[0] as f32, x.v[0] as f32, x.v[0] as f32, 1.0],
                2 => [x.v[0] as f32, x.v[1] as f32, 0.0, 1.0],
                3 => [x.v[0] as f32, x.v[1] as f32, x.v[2] as f32, 1.0],
                _ => [x.v[0] as f32, x.v[1] as f32, x.v[2] as f32, x.v[3] as f32],
            },
            Value::Bool(b) => {
                let f = *b as i32 as f32;
                [f, f, f, 1.0]
            }
            Value::Str(_) => [0.0, 0.0, 0.0, 1.0],
        }
    };
    // the first run reports a broken function as an error instead of a silent black image
    let first = prog.run(&env, [0.5, 0.5]).map_err(|e| Error::BadParam(id, format!("{fname}: {e}")))?;
    if matches!(first, Value::Str(_)) {
        return Err(Error::BadParam(id, format!("{fname}: the result must be a number, vector or boolean")));
    }
    if once {
        return Ok(Computed::Rgba(Tex::solid(w, h, to_px(&first))));
    }
    if let Some(accelerator) = &ctx.ec.accelerator {
        use crate::accelerator::{PixelOutput, PixelRequest};
        match accelerator.pixel(PixelRequest { program: &prog, environment: &env, size: [w, h], cancel: &ctx.ec.cancel }) {
            Ok(Some(image)) => {
                if let Some(profiler) = &ctx.ec.profiler {
                    profiler.accelerated_node(id, &n.kind);
                }
                return Ok(match image {
                    PixelOutput::Grey(image) => Computed::Grey(image, 1.0),
                    PixelOutput::Rgba(image) => Computed::Rgba(image),
                });
            }
            Ok(None) => {}
            Err(_) if ctx.ec.is_cancelled() => return Err(Error::Cancelled),
            Err(reason) => return Err(Error::BadParam(id, reason)),
        }
    }
    // each worker keeps its registers, so what doesn't change from pixel to pixel is computed once
    let new_frame = || {
        let mut f = prog.frame();
        prog.bind(&env, &mut f);
        f
    };
    // one number at every pixel: grey. The first pixel tells; a pixel that gives anything else (a function whose result
    // type changes from pixel to pixel) sends it back to RGBA
    let one = |v: &Value| match v {
        Value::Num(x) => x.lanes == 1,
        Value::Bool(_) | Value::Str(_) => true,
    };
    if let Some(steps) = grey
        && one(&first)
    {
        let other = std::sync::atomic::AtomicBool::new(false);
        let g = crate::tex::grey_from_fn_with(
            w,
            h,
            new_frame,
            |frame, u, v| match prog.run_bound(&env, [u as f64, v as f64], frame) {
                Ok(r) => {
                    if !one(&r) {
                        other.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    to_px(&r)[0]
                }
                Err(_) => 0.0,
            },
            steps,
        );
        if !other.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(match g {
                crate::tex::GreyOut::F(g, a) => Computed::Grey(g, a),
                crate::tex::GreyOut::Q(g) => Computed::Stepped(g),
            });
        }
    }
    Ok(Computed::Rgba(Tex::from_fn_with(w, h, new_frame, |frame, u, v| match prog.run_bound(&env, [u as f64, v as f64], frame) {
        Ok(r) => to_px(&r),
        Err(_) => [0.0, 0.0, 0.0, 1.0],
    })))
}

fn run_sub(id: u32, n: &Node, ins: &BTreeMap<String, Val>, ctx: &Ctx, out: Option<&str>) -> Result<Val, Error> {
    if ctx.depth >= 8 {
        return Err(Error::BadParam(id, "subgraph nesting too deep (recursive?)".into()));
    }
    let name = n.strs.get("graph").ok_or_else(|| Error::BadParam(id, "subgraph needs graph=\"name\"".into()))?;
    let mut sub = ctx.lib.get(name).cloned().ok_or_else(|| Error::BadParam(id, format!("unknown subgraph '{name}'")))?;
    sub.check_cycles()?;
    for (k, v) in &n.nums {
        if let Some((nid, param)) = k.split_once('.')
            && let Ok(nid) = nid.parse::<u32>()
            && let Some(inner) = sub.nodes.get_mut(&nid)
        {
            inner.nums.insert(param.to_string(), *v);
        }
    }
    let ext: Vec<Option<Val>> = (0..4).map(|i| ins.get(&format!("in{i}")).cloned()).collect();
    let mut outs = sub.output_nodes();
    outs.sort_by_key(|o| OUTPUT_USAGES.iter().position(|x| *x == o.0).unwrap_or(99));
    let want = out.map(String::from).or_else(|| n.strs.get("output").cloned()).unwrap_or_default();
    let found = outs.iter().find(|o| !want.is_empty() && (o.0 == want || o.1 == want));
    if out.is_some() && found.is_none() {
        return Err(Error::BadParam(id, format!("graph '{name}' has no output '{want}'")));
    }
    let out_id = found
        .or_else(|| outs.iter().find(|o| o.0 == "out"))
        .or_else(|| outs.first())
        .map(|o| o.2)
        .ok_or_else(|| Error::BadParam(id, format!("subgraph '{name}' has no output")))?;
    let gin = crate::func::graph_inputs(&sub);
    let c2 = Ctx {
        resident: ctx.resident,
        size: ctx.size,
        height: ctx.height,
        nominal: ctx.nominal,
        graph: None,
        lib: ctx.lib,
        res: ctx.res,
        so: ctx.so,
        depth: ctx.depth + 1,
        gin: &gin,
        instance: None,
        plan: ctx.plan,
        ec: ctx.ec,
    };
    let mut cache = Cache::for_graph(&sub, &[out_id], false, &ctx.ec.live);
    let t = eval_node(&sub, out_id, &c2, &ext, &mut cache)?;
    cache.map.clear();
    t.into_host(id, ctx.ec)
}

fn compute(id: u32, n: &Node, ins: &BTreeMap<String, Rc<Tex>>, ctx: &Ctx) -> Result<Tex, Error> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let tex = |slot: &str| -> Result<&Tex, Error> { ins.get(slot).map(|r| r.as_ref()).ok_or_else(|| Error::MissingInput(id, slot.to_string())) };
    let seed = (f("seed", 0.0).max(0.0) as u32).wrapping_add(ctx.so);
    // Tile offset: the whole part picks another tile of the infinite noise plane, the fraction pans the result
    let (tx, ty) = (f("tile_x", 0.0), f("tile_y", 0.0));
    let seed = if tx.floor() != 0.0 || ty.floor() != 0.0 { noise::tile_seed(seed, tx.floor() as i32, ty.floor() as i32) } else { seed };
    let per = |k: &str, d: f32| f(k, d).round().clamp(1.0, 4096.0) as i32;
    let s = ctx.size;
    let hs = ctx.height;
    let rs = s as f32 / 512.0;
    if let Some(out) = compute_generator(n, [s, hs], seed) {
        return Ok(fit_node_output(out, [s, hs], [tx, ty]));
    }
    let out = match n.kind.as_str() {
        "color" => {
            if f("gray", 0.0) > 0.5 {
                let v = f("v", 0.5);
                Tex::solid(s, hs, [v, v, v, f("a", 1.0)])
            } else {
                Tex::solid(s, hs, [f("r", 0.5), f("g", 0.5), f("b", 0.5), f("a", 1.0)])
            }
        }
        "output" => match ins.get("in") {
            Some(t) => (**t).clone(),
            None => Tex::solid(s, hs, output_default(n.strs.get("usage").map(String::as_str).unwrap_or("basecolor"))),
        },
        "dot" => tex("in")?.clone(),
        "tile_sampler" | "tile_generator" => {
            let ar = cell_aspect(n, s, hs);
            let r#gen = n.kind == "tile_generator";
            let mut p = scatter::TileP::legacy(r#gen);
            p.nx = f("x_amount", p.nx);
            p.ny = f("y_amount", p.ny);
            if ar != 1.0 {
                p.ny = (p.nx * ar).round().max(1.0);
            }
            p.size = f("size", p.size);
            p.aspect = f("aspect", 1.0);
            p.size_mode = f("size_mode", 0.0) as u32;
            p.scale = f("scale", 1.0);
            p.scale_rand = f("scale_random", 0.0);
            p.scale_seed = f("scale_seed", 0.0) as u32;
            p.size_rand = f("size_random", 0.0);
            p.rot = f("rotation", 0.0);
            p.rot_rand = f("rotation_random", 0.0);
            p.pattern_rot90 = f("pattern_rotation", 0.0) as u32;
            p.symmetry_random = f("symmetry_random", 0.0) as u32;
            p.pos_rand = f("position_random", 0.0);
            p.pos_rand_x = f("position_random_x", 1.0);
            p.pos_rand_y = f("position_random_y", 1.0);
            p.color = f("color", 1.0);
            p.color_rand = f("color_random", 0.0);
            p.lum_by_number = f("lum_by_number", 0.0);
            p.lum_by_scale = f("lum_by_scale", 0.0);
            p.offset = f("offset", 0.5);
            p.offset_mode = f("offset_mode", p.offset_mode as f32) as u32;
            p.offset_rand = f("offset_random", 0.0);
            p.offset_seed = f("offset_seed", 0.0) as u32;
            p.global_offset = [f("global_offset_x", 0.0), f("global_offset_y", 0.0)];
            p.seed = seed;
            p.thresh = f("mask_threshold", 0.5);
            p.mask_invert = f("mask_invert", 0.0) > 0.5;
            p.mask_random = f("mask_random", 0.0);
            p.mask_checker = f("mask_checker", 0.0) as u32;
            p.mask_h = f("mask_horizontal", 0.0) > 0.5;
            p.mask_v = f("mask_vertical", 0.0) > 0.5;
            p.invert_mask = f("invert_mask", 0.0) > 0.5;
            p.scale_map_mult = f("scale_map_multiplier", 1.0);
            p.rotation_map_mult = f("rotation_map_multiplier", 1.0);
            p.displacement_intensity = f("displacement_intensity", 0.5);
            p.displacement_angle = f("displacement_angle", 0.0);
            p.color_map_mult = f("color_map_multiplier", 1.0);
            p.pattern_specific = f("pattern_specific", 0.5);
            p.pattern_specific_rand = f("pattern_specific_random", 0.0);
            p.blend = scatter::Blend::from_index(f("blending", 0.0) as u32);
            p.opacity = f("opacity", 1.0);
            p.background = f("background", 0.0);
            p.reverse = f("reverse_order", 0.0) > 0.5;
            let pats = scatter_patterns(ins, f("pattern_type", 0.0) as u32, if r#gen { 1 } else { 2 }, f("pattern_distribution", 0.0) as u32);
            let g = |k: &str| ins.get(k).map(|t| t.as_ref());
            let maps = scatter::TileMaps {
                mask: g("mask"),
                scale: g("scale_map"),
                rotation: g("rotation_map"),
                displacement: g("displacement_map"),
                color: g("color_map"),
                distribution: g("distribution_map"),
                background: g("background"),
            };
            scatter::tile_grid(s, s, &pats, &maps, &p)
        }
        "splatter" => {
            let mut p = scatter::SplatterP::legacy();
            p.count = f("count", 40.0).max(0.0) as u32;
            p.grid = f("grid_number", 0.0).max(0.0) as u32;
            p.scale = f("scale", 0.1);
            p.scale_var = f("scale_var", 0.3);
            p.aspect = f("aspect", 1.0);
            p.rot = f("rotation", 0.0);
            p.rot_var = f("rot_var", 1.0);
            p.pattern_rot90 = f("pattern_rotation", 0.0) as u32;
            p.lum_var = f("lum_var", 0.0);
            p.disorder = f("disorder", 1.0);
            p.pan = [f("pan_x", 0.0), f("pan_y", 0.0)];
            p.symmetry_random = f("symmetry_random", 0.0) as u32;
            p.pattern_specific = f("pattern_specific", 0.5);
            p.gain = f("gain", 1.0);
            p.out_min = f("output_min", 0.0);
            p.out_max = f("output_max", 1.0);
            p.background = f("background", 0.0);
            p.blend = scatter::Blend::from_index(f("blending", 0.0) as u32);
            p.opacity = f("opacity", 1.0);
            p.seed = seed;
            let pats = scatter_patterns(ins, f("pattern_type", 0.0) as u32, 99, f("pattern_distribution", 0.0) as u32);
            match f("_out", 0.0).round() as i32 {
                1 => scatter::splatter_data(s, hs, &pats, &p, scatter::SplatData::Id),
                2 => scatter::splatter_data(s, hs, &pats, &p, scatter::SplatData::Center),
                3 => scatter::splatter_data(s, hs, &pats, &p, scatter::SplatData::Size),
                4 => scatter::splatter_data(s, hs, &pats, &p, scatter::SplatData::Luminance),
                _ => scatter::splatter(s, s, &pats, &p),
            }
        }
        "splatter_circular" => {
            let mut p = scatter::CircularP::legacy();
            p.amount = f("number", 12.0).max(1.0) as u32;
            p.amount_rand = f("amount_random", 0.0);
            p.amount_rand_min = f("amount_random_min", 1.0).max(1.0) as u32;
            p.rings = f("rings", 1.0).max(1.0) as u32;
            p.radius = f("radius", 0.3);
            p.radius_rand = f("radius_random", 0.0);
            p.ring_mult = f("ring_radius_multiplier", 0.5);
            p.angle_rand = f("angle_random", 0.0);
            p.spiral = f("spiral_factor", 0.0);
            p.spread = f("spread", 1.0);
            p.size = f("size", 0.1);
            p.size_rand = f("size_random", 0.0);
            p.scale_by_number = f("scale_by_number", 0.0);
            p.invert_number = f("invert_number", 0.0) > 0.5;
            p.scale_by_ring = f("scale_by_ring", 0.0);
            p.invert_ring = f("invert_ring", 0.0) > 0.5;
            p.rot = f("rotation", 0.0);
            p.rot_rand = f("rotation_random", 0.0);
            p.center_orientation = f("center_orientation", 0.0);
            p.ring_rot = f("ring_rotation", 0.0);
            p.ring_rot_rand = f("ring_rotation_random", 0.0);
            p.ring_rot_offset = f("ring_rotation_offset", 0.0);
            p.pos_rand = f("position_random", 0.0);
            p.symmetry_random = f("symmetry_random", 0.0) as u32;
            p.pattern_specific = f("pattern_specific", 0.5);
            p.color = f("color", 1.0);
            p.lum_rand = f("luminance_random", 0.0);
            p.lum_by_number = f("lum_by_number", 0.0);
            p.lum_by_ring = f("lum_by_ring", 0.0);
            p.mask_random = f("mask_random", 0.0);
            p.background = f("background", 0.0);
            p.blend = scatter::Blend::from_index(f("blending", 0.0) as u32);
            p.opacity = f("opacity", 1.0);
            p.seed = seed;
            let pats = scatter_patterns(ins, f("pattern_type", 0.0) as u32, 2, f("pattern_distribution", 0.0) as u32);
            scatter::splatter_circular(s, s, &pats, &p)
        }
        "switch" => {
            let order = ["a", "b", "c", "d"];
            let idx = f("index", 0.0).max(0.0) as usize;
            match order.get(idx.min(3)).and_then(|k| ins.get(*k)) {
                Some(t) => (**t).clone(),
                None => order.iter().find_map(|k| ins.get(*k)).map(|t| (**t).clone()).unwrap_or_else(|| Tex::solid(s, hs, [0.0, 0.0, 0.0, 1.0])),
            }
        }
        "flood_fill" => ops::flood_fill(tex("in")?, f("threshold", 0.05)),
        "flood_fill_to_index" => ops::flood_fill_to_index(tex("in")?),
        "flood_fill_to_position" | "flood_fill_to_bbox_size" => {
            let g = ops::flood_fill_to_geometry(tex("in")?, n.kind == "flood_fill_to_bbox_size");
            match f("_out", 0.0).round() as i32 {
                1 => g.map(|p| [p[0], p[0], p[0], 1.0]),
                2 => g.map(|p| [p[1], p[1], p[1], 1.0]),
                _ => g,
            }
        }
        "flood_fill_random" => ops::flood_fill_random(tex("in")?, seed, f("min", 0.0), f("max", 1.0)),
        "flood_fill_color" => ops::flood_fill_color(tex("in")?, seed),
        "flood_fill_gradient" => ops::flood_fill_gradient(tex("in")?, f("angle", 0.0), f("variation", 0.0), seed),
        "vector_warp" => ops::vector_warp_ex(tex("in")?, tex("vector")?, f("intensity", 0.05), f("directx", 0.0) > 0.5),
        "slope_blur" => ops::slope_blur(tex("in")?, tex("slope")?, f("samples", 16.0) as u32, f("intensity", 0.1), f("mode", 0.0) as u32),
        "blur_hq" => {
            let q = f("quality", 1.0).round().clamp(1.0, 4.0) as u32;
            let mut t = tex("in")?.clone();
            let sigma = f("intensity", 6.0) * rs / (q as f32).sqrt();
            for _ in 0..q {
                t = ops::blur(&t, sigma);
            }
            t
        }
        "histogram_range" => {
            let (pos, range, con2, inv2) = (f("position", 0.5), f("range", 0.2).max(1e-4), f("contrast", 0.5), f("invert", 0.0) > 0.5);
            let soft = (1.0 - con2.clamp(0.0, 1.0)) * range * 0.5 + 1e-4;
            let (lo, hi) = (pos - range * 0.5, pos + range * 0.5);
            tex("in")?.map(move |p| {
                let l = lum(p);
                let g = smooth(lo - soft, lo + soft, l) * (1.0 - smooth(hi - soft, hi + soft, l));
                let g = if inv2 { 1.0 - g } else { g };
                [g, g, g, 1.0]
            })
        }
        "bevel" => {
            let b = ops::bevel_ex(tex("in")?, f("distance", 0.05) * s as f32, f("smoothing", 0.5), f("threshold", 0.5), f("corner", 1.0) < 0.5);
            if f("use_as_normal", 0.0) > 0.5 { ops::normal_from_height(&b, f("normal_intensity", 4.0), f("directx", 0.0) > 0.5) } else { b }
        }
        "distance" => ops::distance(tex("in")?, f("distance", 0.1) * s as f32, f("threshold", 0.5), f("mode", 0.0) as u32),
        "color_match" if f("method", 0.0) < 0.5 => {
            if f("_out", 0.0) >= 0.5 {
                Tex::solid(s, hs, [1.0; 4])
            } else {
                ops::color_match(tex("in")?, ins.get("reference").map(|r| r.as_ref()).unwrap_or(tex("in")?), f("strength", 1.0))
            }
        }
        "color_match" => color_swap_node(f, tex)?,
        "curvature_smooth" => ops::curvature_from_normal(tex("in")?, f("smoothing", 2.0) * rs, f("intensity", 6.0)),
        "height_blend" => {
            let (h, m) = ops::height_blend(tex("bg")?, tex("fg")?, ins.get("mask").map(|x| x.as_ref()), f("opacity", 1.0), f("contrast", 0.5));
            if f("output", 0.0) > 0.5 { m } else { h }
        }
        "make_it_tile_photo" => ops::make_tile_photo_ex(tex("in")?, f("width", 0.6), f("width_v", f("width", 0.6))),
        "make_it_tile_patch" => ops::make_tile_patch(tex("in")?, f("patches", 6.0), f("softness", 0.5), seed),
        "clamp" => {
            let (lo, hi) = (f("low", 0.0), f("high", 1.0));
            let ca = f("apply_alpha", 0.0) > 0.5;
            tex("in")?.map(move |p| {
                [
                    p[0].clamp(lo.min(hi), hi.max(lo)),
                    p[1].clamp(lo.min(hi), hi.max(lo)),
                    p[2].clamp(lo.min(hi), hi.max(lo)),
                    if ca { p[3].clamp(lo.min(hi), hi.max(lo)) } else { p[3] },
                ]
            })
        }
        "highpass" => ops::highpass(tex("in")?, f("radius", 6.0) * rs),
        "swirl" => ops::swirl(tex("in")?, f("amount", 0.3), f("radius", 0.5)),
        "edge_wear" | "dirt" | "dust" => {
            let base = tex("in")?;
            let amount = f("amount", 0.5).clamp(0.0, 1.0);
            let soft = 0.25 / f("contrast", 1.0).max(0.1);
            let thr = 0.5 + (1.0 - amount) * 0.35;
            let mut m = match n.kind.as_str() {
                "edge_wear" => {
                    let c = ops::curvature(base, f("width", 2.0) * rs, 6.0);
                    ops::threshold(&c, thr, soft)
                }
                "dirt" => {
                    let inv = ops::invert(&ops::to_gray(base));
                    ops::threshold(&inv, 1.0 - amount * 0.6, soft)
                }
                _ => ops::threshold(&ops::channel(base, 1), 0.95 - amount * 0.45, soft),
            };
            if let Some(nz) = ins.get("noise") {
                for (p, q) in m.px.iter_mut().zip(nz.px.iter()) {
                    let k = lum(*q);
                    p[0] *= k;
                    p[1] *= k;
                    p[2] *= k;
                }
            }
            m
        }
        "fx_map" => compute_fx_map(id, n, ins, ctx, seed)?,
        "curve" => {
            let get = |k: &str| -> Result<crate::curve::Curve, Error> {
                match n.strs.get(k) {
                    Some(t) if !t.trim().is_empty() => crate::curve::Curve::parse(t).map_err(|m| Error::BadParam(id, format!("{k}: {m}"))),
                    _ => Ok(crate::curve::Curve::identity()),
                }
            };
            let (m, r, g, b, a) = (get("points")?, get("points_r")?, get("points_g")?, get("points_b")?, get("points_a")?);
            crate::curve::apply(tex("in")?, &m, [&r, &g, &b, &a])
        }
        "svg" => {
            let file = n.strs.get("file").ok_or_else(|| Error::BadParam(id, "svg needs file=\"art.svg\"".into()))?;
            let bytes = read_resource(id, file, ctx)?;
            let src = String::from_utf8_lossy(&bytes);
            crate::vector::render_svg(&src, s, hs, f("fit", 0.0) as u32, f("scale", 1.0).max(0.001), f("x", 0.0), f("y", 0.0))
                .map_err(|m| Error::BadParam(id, format!("'{file}': {m}")))?
        }
        "text" => compute_text(id, n, ctx)?,
        "value_processor" => {
            let fname = n.strs.get("fn").ok_or_else(|| Error::BadParam(id, "value_processor needs fn=\"function\"".into()))?;
            return run_function_image(id, n, fname, ins, ctx, true);
        }
        "input_value" => {
            let v = f("v", 0.0);
            Tex::solid(ctx.size, ctx.height, [v, v, v, 1.0])
        }
        "pixel_processor" => {
            if let Some(fname) = n.strs.get("fn") {
                return run_function_image(id, n, fname, ins, ctx, false);
            }
            let parse = |k: &str| -> Result<Option<expr::Ex>, Error> {
                match n.strs.get(k) {
                    Some(src) => expr::parse(src).map(Some).map_err(|m| Error::BadParam(id, format!("{k}: {m}"))),
                    None => Ok(None),
                }
            };
            let e0 = parse("expr")?.ok_or_else(|| Error::BadParam(id, "missing expr".into()))?;
            let (eg, eb, ea) = (parse("expr_g")?, parse("expr_b")?, parse("expr_a")?);
            let input = ins.get("in").map(|t| t.as_ref());
            Tex::from_fn(s, hs, |u, v| {
                let c = input.map(|t| t.sample(u, v)).unwrap_or([0.0, 0.0, 0.0, 1.0]);
                let env = expr::Env { vars: [u, v, u * s as f32, v * s as f32, s as f32, s as f32, c[0], c[1], c[2], lum(c)], tex: input };
                let r = expr::eval(&e0, &env);
                let g = eg.as_ref().map(|e| expr::eval(e, &env)).unwrap_or(r);
                let b = eb.as_ref().map(|e| expr::eval(e, &env)).unwrap_or(r);
                let a = ea.as_ref().map(|e| expr::eval(e, &env)).unwrap_or(1.0);
                [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a.clamp(0.0, 1.0)]
            })
        }
        "sdf_shape" => {
            let (shape, size, aspect, sides, rot, round) =
                (f("shape", 0.0) as u32, f("size", 0.25), f("aspect", 1.0), f("sides", 5.0), f("rotation", 0.0), f("round", 0.0));
            let (cx, cy, range) = (f("x", 0.0), f("y", 0.0), f("range", 0.25));
            Tex::gray_fn(s, hs, move |u, v| {
                ops::sdf_to_value(ops::sdf2d(shape, ops::wrapd(u - 0.5 - cx), ops::wrapd(v - 0.5 - cy), size, aspect, sides, rot, round), range)
            })
        }
        "sdf_op" => {
            let (op, k, range) = (f("op", 0.0) as u32, f("smooth", 0.0), f("range", 0.25));
            let (a, b) = (tex("a")?, tex("b")?);
            Tex::gray_fn(s, hs, move |u, v| {
                let (da, db) = (ops::value_to_sdf(lum(a.sample(u, v)), range), ops::value_to_sdf(lum(b.sample(u, v)), range));
                ops::sdf_to_value(ops::sdf_combine(op, da, db, k), range)
            })
        }
        "sdf_mask" => {
            let (off, soft) = (f("offset", 0.0), f("softness", 0.02).max(1e-4));
            let a = tex("in")?;
            Tex::gray_fn(s, hs, move |u, v| smooth(0.5 + off - soft * 0.5, 0.5 + off + soft * 0.5, lum(a.sample(u, v))))
        }
        "sdf_form" => {
            let (shape, size, rep, round, scatter, hgt) =
                (f("shape", 0.0) as u32, f("size", 0.3), f("repeat", 3.0), f("round", 0.05), f("scatter", 0.3), f("height", 1.5));
            Tex::gray_fn(s, hs, move |u, v| ops::sdf_form(u, v, shape, size, rep, round, scatter, seed, hgt))
        }
        "bitmap" => compute_bitmap(id, n, ctx)?,
        "channel_shuffle" => compute_channel_shuffle(tex("in")?, ins, f),
        "blur" => {
            let (src, sg) = (tex("in")?, f("radius", 4.0) * rs);
            ops::blur(src, sg)
        }
        "directional_blur" => {
            let (src, a, l) = (tex("in")?, f("angle", 0.0), f("length", 0.05));
            ops::directional_blur(src, a, l)
        }
        "sharpen" => ops::sharpen(tex("in")?, f("radius", 2.0) * rs, f("amount", 1.0)),
        "levels" => {
            let t = tex("in")?;
            let (mut lo, mut hi) = (f("low", 0.0), f("high", 1.0));
            if f("auto", 0.0) > 0.5 {
                let (mn, mx) = t.px.iter().fold((1.0f32, 0.0f32), |a, p| (a.0.min(lum(*p)), a.1.max(lum(*p))));
                lo = mn;
                hi = mx.max(mn + 1e-4);
            }
            let (gm, ol, oh) = (f("gamma", 1.0), f("out_low", 0.0), f("out_high", 1.0));
            ops::levels(t, lo, hi, gm, ol, oh)
        }
        "adjust" => ops::adjust(tex("in")?, f("brightness", 0.0), f("contrast", 1.0)),
        "invert" => {
            if f("invert", 1.0) > 0.5 {
                ops::invert(tex("in")?)
            } else {
                tex("in")?.clone()
            }
        }
        "threshold" => ops::threshold_mode(tex("in")?, f("threshold", 0.5), f("softness", 0.0), f("mode", 0.0).max(0.0) as u32),
        "histogram_scan" => {
            let t = ops::histogram_scan(tex("in")?, f("position", 0.5), (1.0 - f("contrast", 0.5).clamp(0.0, 1.0)).max(1e-4));
            if f("invert", 0.0) > 0.5 { ops::invert(&t) } else { t }
        }
        "quantize" => ops::quantize(tex("in")?, f("steps", 4.0)),
        "to_gray" => ops::to_gray(tex("in")?),
        "channel" => ops::channel(tex("in")?, f("ch", 0.0).max(0.0) as usize),
        "merge" => {
            let g = |slot: &str, ch: usize| ins.get(slot).map(|t| ops::channel(t, ch));
            let (r, gg, b, a) = (g("r", 0), g("g", 1), g("b", 2), g("a", 3));
            Tex::from_fn(s, hs, |u, v| {
                let q = |t: &Option<Tex>, d: f32| t.as_ref().map(|t| t.sample(u, v)[0]).unwrap_or(d);
                [q(&r, 0.0), q(&gg, 0.0), q(&b, 0.0), q(&a, 1.0)]
            })
        }
        "tint" => {
            let c = [f("r", 1.0), f("g", 1.0), f("b", 1.0)];
            tex("in")?.map(move |p| [p[0] * c[0], p[1] * c[1], p[2] * c[2], p[3]])
        }
        "colorize" => colorize(n, tex("in")?).ok_or_else(|| Error::BadParam(id, "colorize needs at least one stop (p0 r0 g0 b0)".into()))?,
        "hsl" => ops::hsl_adjust(tex("in")?, f("hue", 0.0), f("sat", 1.0), f("val", 1.0)),
        "transform" => ops::transform(
            tex("in")?,
            f("offset_x", 0.0),
            f("offset_y", 0.0),
            f("scale_x", 1.0),
            f("scale_y", 1.0),
            f("rotation", 0.0),
            f("tiling", 0.0) < 0.5,
        ),
        "mirror" => ops::mirror_ex(
            tex("in")?,
            f("axis", 0.0) as u32,
            f("offset_x", 0.5),
            f("offset_y", 0.5),
            f("invert_x", 0.0) > 0.5,
            f("invert_y", 0.0) > 0.5,
            f("corner", 0.0) as u32,
        ),
        "warp" => ops::warp_ex(tex("in")?, tex("warp")?, f("intensity", 0.05), f("filtering", 2.0).max(0.0) as u32),
        "directional_warp" => ops::directional_warp_ex(
            tex("in")?,
            tex("warp")?,
            f("angle", 0.0),
            f("intensity", 0.1),
            f("offset", 0.5),
            f("filtering", 2.0).max(0.0) as u32,
        ),
        "blend" => {
            let (fg, bg, mask) = (tex("fg")?, tex("bg")?, ins.get("mask").map(|m| m.as_ref()));
            let (mode, op, am) = (f("mode", 0.0).max(0.0) as u32, f("opacity", 1.0), f("alpha_mode", 1.0).max(0.0) as u32);
            let crop = [f("crop_x0", 0.0), f("crop_y0", 0.0), f("crop_x1", 1.0), f("crop_y1", 1.0)];
            ops::blend_ex(fg, bg, mask, mode, op, am, crop)
        }
        "normal" => height_to_normal_node(tex("in")?, f),
        "normal_blend" => ops::normal_blend_ex(tex("a")?, tex("b")?, f("opacity", 1.0), ins.get("mask").map(|m| m.as_ref())),
        "ao" if f("mode", 0.0) < 0.5 => ops::ambient_occlusion(tex("in")?, f("radius", 6.0) * rs, f("intensity", 2.0)),
        "ao" => {
            let scale =
                if f("use_world_units", 0.0) > 0.5 { f("height_depth_cm", 30.0) / f("surface_size", 300.0).max(1.0) } else { f("height_depth", 0.1) };
            let dirs = [4u32, 8, 16].get((f("quality", 1.0).round() as usize).min(2)).copied().unwrap_or(16);
            ops::hbao(tex("in")?, f("hbao_radius", 1.0).clamp(0.0, 1.0) * 0.25, scale, dirs, f("hbao_intensity", 1.0))
        }
        "quantize_color" => quantize_color_node(tex("in")?, [s, hs], per("color_amount", 32.0).clamp(1, 128) as usize, f),
        "rgba_split" => {
            let c = (f("_out", 0.0).round() as usize).min(3);
            tex("in")?.map(|p| {
                let v = p.get(c).copied().unwrap_or(0.0);
                [v, v, v, 1.0]
            })
        }
        "alpha_split" => {
            if f("_out", 0.0) >= 0.5 {
                tex("in")?.map(|p| [p[3], p[3], p[3], 1.0])
            } else {
                tex("in")?.map(|p| [p[0], p[1], p[2], 1.0])
            }
        }
        "curvature" => {
            if f("mode", 0.0) > 0.5 {
                ops::curvature(tex("in")?, f("radius", 3.0) * rs, f("intensity", 4.0))
            } else {
                ops::curvature_normal(tex("in")?, f("intensity", 4.0), f("directx", 0.0) > 0.5)
            }
        }
        "edge_detect" => {
            if f("mode", 0.0) > 0.5 {
                ops::edge_detect(tex("in")?, f("intensity", 2.0))
            } else {
                // widths are given in pixels at the 512 reference size and scale with the resolution
                ops::edge_detect_ex(
                    tex("in")?,
                    (f("edge_width", 2.0) * rs).max(1.0),
                    f("edge_roundness", 4.0) * rs,
                    f("invert", 0.0) > 0.5,
                    f("tolerance", 0.0),
                )
            }
        }
        "emboss" => ops::emboss(tex("in")?, f("angle", 0.125), f("intensity", 4.0)),
        _ => compute_other(id, n, ins, ctx, seed)?,
    };
    Ok(fit_node_output(out, [s, hs], [tx, ty]))
}

fn height_to_normal_node(src: &Tex, f: impl Fn(&str, f32) -> f32) -> Tex {
    let (st, dx) = (f("strength", 2.0), f("directx", 0.0) > 0.5);
    let n = ops::normal_from_height(src, st, dx);
    if f("alpha_from_input", 0.0) > 0.5 {
        // "Fill alpha with input": the height map goes into the alpha channel
        let mut n = n;
        for (o, i) in n.px.iter_mut().zip(&src.px) {
            o[3] = crate::tex::lum(*i);
        }
        n
    } else {
        n
    }
}

fn quantize_color_node(src: &Tex, [s, hs]: [usize; 2], colors: usize, f: impl Fn(&str, f32) -> f32) -> Tex {
    let rs = s as f32 / 512.0;
    let (res, ids, pal) = ops::quantize_color_full(
        src,
        colors,
        f("contour_smoothing", 0.0) * rs,
        f("dithering", 0.0).clamp(0.0, 1.0),
        f("dithering_pattern", 0.0) > 0.5,
        f("distance_color_space", 0.0) < 0.5,
        f("ignore_alpha", 0.0) > 0.5,
    );
    match f("_out", 0.0).round() as i32 {
        1 => {
            // ID: every region's palette index as a gray level (index / (colours - 1))
            let div = (pal.len().max(2) - 1) as f32;
            let mut t = res.clone();
            for (o, i) in t.px.iter_mut().zip(&ids) {
                let v = *i as f32 / div;
                *o = [v, v, v, 1.0];
            }
            t
        }
        2 => {
            // Palette: one vertical band per colour, left to right in index order
            let k = pal.len();
            Tex::from_fn(s, hs, move |u, _| {
                let c = pal.get(((u * k as f32) as usize).min(k.saturating_sub(1))).copied().unwrap_or_default();
                [c[0], c[1], c[2], 1.0]
            })
        }
        _ => res,
    }
}

fn compute_other(id: u32, n: &Node, ins: &BTreeMap<String, Rc<Tex>>, ctx: &Ctx, seed: u32) -> Result<Tex, Error> {
    let kind = n.kind.as_str();
    if let Some(t) = crate::nodes2::compute(kind, id, n, ins, ctx.size, ctx.height, seed)? {
        return Ok(t);
    }
    let pctx = crate::patterns::Context { lib: ctx.lib, size: [ctx.size, ctx.height], graph_inputs: ctx.gin, seed: ctx.so, graph: ctx.graph };
    crate::atomic::compute(kind, id, n, ins, &pctx)?.ok_or_else(|| Error::UnknownNode(id, kind.to_owned()))
}

fn fit_node_output(out: Tex, [s, hs]: [usize; 2], [tx, ty]: [f32; 2]) -> Tex {
    let out = if out.w != s || out.h != hs { ops::resize_to(&out, s, hs) } else { out };
    let (fx, fy) = (tx - tx.floor(), ty - ty.floor());
    if fx > 0.0 || fy > 0.0 {
        return ops::roll(&out, fx, fy);
    }
    out
}

// `square_cells` keeps lattice cells square in a non-square graph: the vertical count follows the aspect ratio
fn cell_aspect(n: &Node, width: usize, height: usize) -> f32 {
    if n.nums.get("square_cells").copied().unwrap_or(0.0) > 0.5 { height as f32 / width as f32 } else { 1.0 }
}

fn compute_generator(n: &Node, [s, hs]: [usize; 2], seed: u32) -> Option<Tex> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let per = |k: &str, d: f32| f(k, d).round().clamp(1.0, 4096.0) as i32;
    let ar = cell_aspect(n, s, hs);
    let per_y = |k: &str, d: f32| ((per(k, d) as f32 * ar).round().max(1.0)) as i32;
    // one pixel in uv units: hard edges are softened by at least this much, so they are anti-aliased at any resolution
    let px1 = 1.2 / s as f32;
    let (dis, con, inv) = (f("disorder", 0.0), f("contrast", 1.0), f("invert", 0.0) > 0.5);
    let dz = noise::Disorder::new(seed, dis, f("disorder_speed", 1.0), f("anisotropy", 0.0), f("anisotropy_angle", 0.0));
    let warp = move |u: f32, v: f32| -> (f32, f32) { dz.warp(u, v) };
    let fin = |t: Tex| -> Tex { if (con - 1.0).abs() > 1e-4 || inv { ops::contrast_invert(&t, con, inv) } else { t } };
    Some(match n.kind.as_str() {
        "white_noise" => {
            // Noise distribution: 0 uniform, 1 gaussian (bell curve), 2 triangle
            let dist = f("distribution", 0.0).round() as u32;
            Tex::gray_fn(s, hs, move |u, v| {
                let (x, y) = ((u * s as f32) as i32, (v * s as f32) as i32);
                match dist {
                    1 => {
                        // mean of four uniforms: a bell curve around 0.5 (std about 0.14)
                        let sum: f32 = (0..4).map(|i| noise::rnd(x, y, seed.wrapping_add(i * 17))).sum();
                        sum / 4.0
                    }
                    2 => (noise::rnd(x, y, seed) + noise::rnd(x, y, seed.wrapping_add(17))) * 0.5,
                    _ => noise::rnd(x, y, seed),
                }
            })
        }
        "gaussian_noise" => {
            let k = f("scale", 1.0).round().max(1.0) as i32;
            fin(Tex::gray_fn(s, hs, move |u, v| {
                // displaced positions wrap around so the noise still tiles
                let (a, b) = warp(u, v);
                let (x, y) = (((a * s as f32).floor() as i32).rem_euclid(s as i32) / k, ((b * s as f32).floor() as i32).rem_euclid(s as i32) / k);
                let sum: f32 = (0..4).map(|i| noise::rnd(x, y, seed.wrapping_add(i * 17))).sum();
                ((sum / 4.0 - 0.5) * 3.46 + 0.5).clamp(0.0, 1.0)
            }))
        }
        "perlin" => {
            let (px, py) = (per("scale_x", f("scale", 4.0)), per_y("scale_y", f("scale", 4.0)));
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (a, b) = warp(u, v);
                noise::perlin(a, b, px, py, seed)
            }))
        }
        "fbm" | "clouds" => {
            let cl = n.kind == "clouds";
            let (px, py) = (per("scale_x", f("scale", if cl { 3.0 } else { 4.0 })), per_y("scale_y", f("scale", if cl { 3.0 } else { 4.0 })));
            let (oct, gain, mode) =
                (f("octaves", if cl { 6.0 } else { 5.0 }).max(1.0) as u32, f("gain", if cl { 0.6 } else { 0.5 }), f("mode", 0.0).max(0.0) as u32);
            let kernel = noise::Fbm::new(px, py, oct, gain, seed, mode);
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (a, b) = warp(u, v);
                kernel.sample(a, b)
            }))
        }
        "bnw_spots" => {
            let spots = noise::BnwSpots { style: f("variant", 0.0) as u32, scale: per("scale", 8.0), seed };
            fin(Tex::gray_fn(s, hs, move |u, v| spots.value_warped(u, v, warp(u, v))))
        }
        "crystal" => {
            let crystal = noise::Crystal { style: f("variant", 0.0) as u32, scale: per("scale", 6.0), seed };
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (a, b) = warp(u, v);
                crystal.value(a, b)
            }))
        }
        "directional_noise" => {
            // Streaks = tileable Perlin noise averaged along the streak direction. Every sample is taken from a
            // periodic function at wrapped coordinates, so the result tiles seamlessly at ANY angle.
            let (sc, st, ang) = (f("scale", 6.0), f("stretch", 8.0).max(1.0), f("angle", 0.0));
            let p1 = sc.round().clamp(1.0, 256.0) as i32;
            let (sa, ca) = math::sin_cos(ang * std::f32::consts::TAU);
            // a streak never needs to be longer than one tile (longer would wrap and average the noise away)
            let len1 = (st / p1 as f32).min(1.0);
            let k = ((st * 2.5).ceil() as usize).clamp(8, 96) | 1;
            let taps: Vec<(f32, f32)> = (0..k)
                .map(|i| {
                    let t = ((i as f32 + 0.5) / k as f32) * 2.0 - 1.0;
                    (t, 1.0 - t.abs())
                })
                .collect();
            let wsum: f32 = taps.iter().map(|t| t.1).sum();
            let gain = 1.6 * math::powf(st.min(p1 as f32), 0.55); // restores the contrast lost by averaging (calibrated)
            let (g1, g2) = (noise::GradGrid::new(p1, p1, seed), noise::GradGrid::new(p1 * 2, p1 * 2, seed.wrapping_add(7)));
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (u, v) = warp(u, v);
                let (mut a1, mut a2) = (0.0f32, 0.0f32);
                for &(t, w) in &taps {
                    let (o1, o2) = (t * len1 * 0.5, t * len1 * 0.25);
                    a1 += w * g1.sample(u + o1 * ca, v + o1 * sa);
                    a2 += w * g2.sample(u + o2 * ca, v + o2 * sa);
                }
                let m = (a1 * 0.65 + a2 * 0.35) / wsum;
                (0.5 + (m - 0.5) * gain).clamp(0.0, 1.0)
            }))
        }
        "grunge" => {
            let mut g = grunge::GrungeVariant::new(grunge::Variant::from_index(f("variant", 0.0) as u32), seed);
            g.balance = f("balance", 0.5).clamp(0.0, 1.0);
            // concrete
            (g.large_variation, g.base_noisiness) = (f("large_variation", 0.55), f("base_noisiness", 0.45));
            (g.dirt_specks, g.scratches, g.sharpen) =
                (f("dirt_specks_opacity", 0.0).clamp(0.0, 1.0), f("scratches_opacity", 0.0).clamp(0.0, 1.0), f("sharpen", 0.0).clamp(0.0, 1.0));
            g.invert_dirt = f("invert_dirt", 0.0) > 0.5;
            // scratches
            (g.quantity, g.scratch_width, g.scratch_length) =
                (per("quantity", 3.0).min(32), f("scratch_width", 0.4).clamp(0.05, 0.95), f("scratch_length", 0.45).clamp(0.0, 0.9));
            // spots
            (g.details, g.coverage, g.coverage_contrast) =
                (per("details", 22.0).min(128), f("coverage", 0.35).clamp(0.0, 0.9), f("coverage_contrast", 0.25).clamp(0.02, 1.0));
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (a, b) = warp(u, v);
                g.value(a, b)
            }))
        }
        "grunge_map" => {
            let gp = grunge::GrungeP {
                map: per("map", 1.0).clamp(1, 15) as u32,
                seed,
                balance: f("balance", 0.5).clamp(0.0, 1.0),
                contrast: f("contrast", 0.5).clamp(0.0, 1.0),
                brush: f("brush_pattern", 0.0).clamp(0.0, 1.0),
                cracks: f("cracks", 0.5).clamp(0.0, 1.0),
                disorder: f("disorder", 0.0).clamp(0.0, 1.0),
            };
            let t0 = f("coverage", grunge::T0.get(gp.map.saturating_sub(1) as usize).copied().unwrap_or(0.5));
            let inv = f("invert", 0.0) > 0.5;
            Tex::gray_fn(s, hs, move |u, v| {
                let x = grunge::grunge_map(&gp, u, v, t0);
                if inv { 1.0 - x } else { x }
            })
        }
        "plasma" => {
            let sc = f("scale", 3.0).round().max(1.0);
            fin(Tex::gray_fn(s, hs, move |u, v| {
                let (a, b) = warp(u, v);
                let t = std::f32::consts::TAU;
                let p = math::sin(t * (sc * a + 0.35 * math::sin(t * (sc + 1.0) * b)))
                    + math::sin(t * (sc * b + 0.35 * math::sin(t * (sc + 2.0) * a)))
                    + math::sin(t * ((sc + 1.0) * (a + b)));
                (0.5 + p / 6.0).clamp(0.0, 1.0)
            }))
        }
        "stripes" => {
            let (cnt, ang, soft, wd) = (f("count", 8.0).max(1.0), f("angle", 0.0), f("softness", 0.1).max(1e-3), f("width", 0.5));
            // tile=1 (default): snap to the nearest seamlessly tileable direction (integer wave vector).
            // tile=0: exact angle and count, but the pattern only tiles when count*cos/sin are whole numbers.
            let (a, b) = if f("tile", 1.0) > 0.5 {
                let (a, b) = ops::tileable_wave(cnt, ang);
                (a as f32, b as f32)
            } else {
                let (sa, ca) = math::sin_cos(ang * std::f32::consts::TAU);
                (cnt.round() * ca, cnt.round() * sa)
            };
            Tex::gray_fn(s, hs, move |u, v| {
                let t = (u * a + v * b).rem_euclid(1.0);
                smooth(0.0, soft, t) * (1.0 - smooth(wd, wd + soft, t))
            })
        }
        "weave" => {
            // even count: the over/under pattern wraps
            let weave = noise::Weave { count: (f("count", 6.0) / 2.0).round().max(1.0) * 2.0, width: f("width", 0.8).clamp(0.1, 1.0) };
            Tex::gray_fn(s, hs, move |u, v| weave.value(u, v))
        }
        "shape" => {
            let (k, sx, sy, rot, px, py, til, so) = (
                f("shape", 1.0) as u32,
                f("size_x", 0.6),
                f("size_y", 0.6),
                f("rotation", 0.0),
                f("pos_x", 0.0),
                f("pos_y", 0.0),
                f("tiling", 1.0),
                f("softness", 0.02),
            );
            Tex::gray_fn(s, hs, move |u, v| ops::shape_value(k, u, v, px, py, sx, sy, rot, til, so))
        }
        "star" => {
            let (pts, inner, size, so, rot) =
                (f("points", 5.0).round().max(3.0), f("inner", 0.45), f("size", 0.4), f("softness", 0.01).max(px1), f("rotation", 0.0));
            Tex::gray_fn(s, hs, move |u, v| {
                ops::tiled_max(size > 0.5, ops::wrapd(u - 0.5), ops::wrapd(v - 0.5), |x, y| {
                    let a = math::atan2(y, x) - rot * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
                    let seg = std::f32::consts::TAU / pts;
                    let t = a.rem_euclid(seg) / seg;
                    let edge = size * inner + (size - size * inner) * (1.0 - (2.0 * t - 1.0).abs());
                    1.0 - smooth(-so, 0.0, (x * x + y * y).sqrt() - edge)
                })
            })
        }
        "gradient_axial" => {
            // direction snapped to a whole wave vector so the symmetric ramp repeats seamlessly
            let (a, b) = ops::tileable_wave(1.0, f("angle", 0.0));
            let (a, b) = (a as f32, b as f32);
            Tex::gray_fn(s, hs, move |u, v| 1.0 - (2.0 * ((u - 0.5) * a + (v - 0.5) * b + 0.5).rem_euclid(1.0) - 1.0).abs())
        }
        "gradient_circular" => {
            // the angular ramp fades to a flat value towards the tile border, so neighbouring tiles meet seamlessly
            let off = f("offset", 0.0);
            Tex::gray_fn(s, hs, move |u, v| {
                let (x, y) = (ops::wrapd(u - 0.5), ops::wrapd(v - 0.5));
                let a = (math::atan2(y, x) / std::f32::consts::TAU + 0.5 + off).rem_euclid(1.0);
                let w = 1.0 - smooth(0.3, 0.5, x.abs().max(y.abs()));
                0.5 + (a - 0.5) * w
            })
        }
        "voronoi" | "voronoi_fractal" => {
            let fractal = n.kind == "voronoi_fractal";
            let mut vs = noise::VoronoiSettings::new(per("scale", 6.0), seed);
            (vs.jitter, vs.style) = (f("jitter", 1.0).clamp(0.0, 1.0), f("mode", 0.0) as u32);
            if !fractal {
                // extra outputs show the same cells as an id map, an edge map and the plain distance
                vs.style = match f("_out", 0.0).round() as i32 {
                    1 => 1,
                    2 => 2,
                    3 => 0,
                    _ => vs.style,
                };
            }
            vs.aspect = (f("aspect", 1.0) * ar).clamp(0.05, 8.0);
            vs.offset = (f("offset_x", 0.0), f("offset_y", 0.0));
            (vs.distortion, vs.distortion_scale) = (f("distortion", 0.0).clamp(0.0, 2.0), per("distortion_scale", 3.0));
            vs.metric = noise::Metric::from_index(f("distance_mode", 0.0) as u32, f("minkowski", 2.0));
            (vs.thickness, vs.rounded, vs.distance_scale) = (f("edge_thickness", 0.25), f("rounded", 0.0), f("distance_scale", 1.0).clamp(0.0, 8.0));
            vs.id_seed = if f("color_seed_mode", 0.0) > 0.5 { f("color_seed", 0.0) as u32 } else { seed };
            (vs.min_level, vs.max_level) = (f("min_level", 0.0).clamp(0.0, 8.0) as u32, f("max_level", 3.0).clamp(0.0, 8.0) as u32);
            (vs.roughness, vs.lacunarity, vs.opacity, vs.blend) = (
                f("roughness", 0.5).clamp(0.0, 1.5),
                f("lacunarity", 2.0).clamp(1.0, 4.0),
                f("opacity", 1.0).clamp(0.0, 1.0),
                f("blend", 0.0) as u32,
            );
            let vf = noise::VoronoiFractal::new(&vs, fractal);
            fin(Tex::gray_fn(s, hs, move |u, v| vf.value(u, v)))
        }
        "gradient" => {
            let (mode, (sa, ca)) = (f("mode", 0.0) as u32, math::sin_cos(f("angle", 0.0) * std::f32::consts::TAU));
            // linear mode: direction snapped to a whole wave vector and folded (0 -> 1 -> 0), so it repeats seamlessly
            let (wa, wb) = ops::tileable_wave(1.0, f("angle", 0.0));
            let (wa, wb) = (wa as f32, wb as f32);
            let _ = (sa, ca);
            Tex::gray_fn(s, hs, move |u, v| {
                if mode == 1 {
                    let (x, y) = (ops::wrapd(u - 0.5), ops::wrapd(v - 0.5));
                    ((x * x + y * y).sqrt() * 2.0).clamp(0.0, 1.0)
                } else {
                    1.0 - (2.0 * ((u - 0.5) * wa + (v - 0.5) * wb + 0.5).rem_euclid(1.0) - 1.0).abs()
                }
            })
        }
        "circle" => {
            let (r, so) = (f("radius", 0.4), f("softness", 0.01).max(px1));
            Tex::gray_fn(s, hs, move |u, v| {
                ops::tiled_max(r > 0.5, ops::wrapd(u - 0.5), ops::wrapd(v - 0.5), |x, y| 1.0 - smooth(r - so, r, (x * x + y * y).sqrt()))
            })
        }
        "rect" => {
            let (w, h, so) = (f("width", 0.6), f("height", 0.4), f("softness", 0.01).max(px1));
            Tex::gray_fn(s, hs, move |u, v| {
                let d = (ops::wrapd(u - 0.5).abs() - w * 0.5).max(ops::wrapd(v - 0.5).abs() - h * 0.5);
                1.0 - smooth(-so, 0.0, d)
            })
        }
        "polygon" => {
            let (sd, r, so, rot) = (f("sides", 5.0), f("radius", 0.4), f("softness", 0.01).max(px1), f("rotation", 0.0));
            Tex::gray_fn(s, hs, move |u, v| {
                ops::tiled_max(r > 0.5, ops::wrapd(u - 0.5), ops::wrapd(v - 0.5), |x, y| 1.0 - smooth(-so, 0.0, ops::polygon_sdf(x, y, sd, r, rot)))
            })
        }
        "checker" => {
            let t = (f("tiles", 8.0) / 2.0).round().max(1.0) * 2.0; // even count so the pattern wraps
            // anti-aliased: blend across a cell border over about one pixel
            let aa = (t / s as f32).max(1e-6);
            Tex::gray_fn(s, hs, move |u, v| {
                let (x, y) = (u * t, v * t);
                let (fx, fy) = (x - x.floor(), y - y.floor());
                let (ix, iy) = (x.floor() as i64, y.floor() as i64);
                // signed distances (in cells) to the nearest vertical / horizontal border and which side we are on
                let sx = ((fx.min(1.0 - fx)) / aa).min(1.0) * 0.5 + 0.5; // 0.5 at the border .. 1 inside
                let sy = ((fy.min(1.0 - fy)) / aa).min(1.0) * 0.5 + 0.5;
                let base = ((ix + iy) & 1) as f32;
                // mix towards the average (0.5) at the border: at a border the neighbour cell has the opposite value
                let kx = 1.0 - sx; // 0 inside .. 0.5 at the border
                let ky = 1.0 - sy;
                let v1 = base + (1.0 - 2.0 * base) * kx; // moves towards the neighbour value across x
                let v2 = v1 + (1.0 - 2.0 * v1) * ky;
                v2.clamp(0.0, 1.0)
            })
        }
        "bricks" => {
            let (r, c, m, b, o, lv) = (f("rows", 8.0), f("cols", 4.0), f("mortar", 0.03), f("bevel", 0.05), f("offset", 0.5), f("lum_var", 0.3));
            let b = b.max(1.5 * r.max(c) / s as f32); // edge ramps are at least about a pixel wide (anti-aliased)
            Tex::gray_fn(s, hs, move |u, v| ops::bricks(u, v, r, c, m, b, o, lv, seed))
        }
        _ => return None,
    })
}

fn color_swap_node<'a>(f: impl Fn(&str, f32) -> f32, tex: impl Fn(&str) -> Result<&'a Tex, Error>) -> Result<Tex, Error> {
    let src = tex("in")?;
    let rgb = |name: &str, d: f32| [f(&format!("{name}_r"), d), f(&format!("{name}_g"), d), f(&format!("{name}_b"), d)];
    let source = match f("source_mode", 0.0).round() as i32 {
        1 => rgb("src", 0.5),
        2 => ops::mean_rgb(tex("src_color")?),
        _ => ops::mean_rgb(src),
    };
    let target = if f("target_mode", 0.0) > 0.5 { ops::mean_rgb(tex("target_color")?) } else { rgb("target", 0.5) };
    let var = f("custom_variation", 0.0) > 0.5;
    let sw = ops::ColorSwap {
        source,
        target,
        hue_shift: if var { f("variation_hue", 0.0) / 360.0 } else { 0.0 },
        chroma_mul: if var { f("variation_chroma", 1.0).max(0.0) } else { 1.0 },
        luma_mul: if var { f("variation_luma", 1.0).max(0.0) } else { 1.0 },
        use_mask: f("use_mask", 1.0) > 0.5,
        hue_range: f("mask_hue_range", 30.0) / 360.0,
        chroma_range: f("mask_chroma_range", 0.5),
        luma_range: f("mask_luma_range", 0.5),
        smooth: f("mask_smoothness", 0.3),
        strength: f("strength", 1.0),
    };
    let (res, mask) = ops::color_swap(src, &sw);
    Ok(if f("_out", 0.0) >= 0.5 { mask } else { res })
}

fn compute_fx_map(id: u32, n: &Node, ins: &BTreeMap<String, Rc<Tex>>, ctx: &Ctx, seed: u32) -> Result<Tex, Error> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let (s, hs) = (ctx.size, ctx.height);
    let name = n.strs.get("program").ok_or_else(|| Error::BadParam(id, "fx_map needs program=\"name\"".into()))?;
    let prog = ctx.lib.get(name).ok_or_else(|| Error::BadParam(id, format!("unknown pattern program '{name}'")))?;
    if !crate::fxmap::is_program(prog) {
        return Err(Error::BadParam(id, format!("'{name}' is not a pattern program")));
    }
    let pats = [
        ins.get("in").map(|t| t.as_ref()),
        ins.get("in2").map(|t| t.as_ref()),
        ins.get("in3").map(|t| t.as_ref()),
        ins.get("in4").map(|t| t.as_ref()),
    ];
    let color = f("color", 0.0) > 0.5;
    let bg = f("bg", 0.0);
    let bgc = if color { [f("bg_r", bg), f("bg_g", bg), f("bg_b", bg)] } else { [bg, bg, bg] };
    let run = crate::fxmap::Run { depth: f("depth", 12.0).clamp(0.0, 32.0) as u32, seed, blend: f("blend", 0.0) as u32, color, patterns: &pats };
    crate::fxmap::render(prog, s, hs, bgc, &run).map_err(|e| Error::BadParam(id, format!("{name}: {e}")))
}

fn compute_text(id: u32, n: &Node, ctx: &Ctx) -> Result<Tex, Error> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let (s, hs) = (ctx.size, ctx.height);
    let font = match n.strs.get("font").filter(|f| !f.is_empty()) {
        Some(file) => crate::font::Font::load(&read_resource(id, file, ctx)?).map_err(|m| Error::BadParam(id, format!("'{file}': {m}")))?,
        None => crate::font::Font::Builtin,
    };
    let text = n.strs.get("text").cloned().unwrap_or_default();
    // compiled font graphs count the position's y up the image
    let y = if f("y_up", 0.0) > 0.5 { -f("y", 0.0) } else { f("y", 0.0) };
    let xf = crate::vector::Xf::rotate(f("rotation", 0.0).to_radians())
        .then(crate::vector::Xf::translate((0.5 + f("x", 0.0)) * s as f32, (0.5 + y) * hs as f32));
    let polys = font.layout(&text, f("size", 0.2).max(0.001) * hs as f32, f("align", 1.0) as u32, f("spacing", 0.0), f("line_spacing", 1.0), &xf);
    let mask = crate::vector::fill(&polys, s, hs, false);
    let v = f("value", 1.0);
    let mut t = Tex::solid(s, hs, [0.0, 0.0, 0.0, 1.0]);
    for (p, c) in t.px.iter_mut().zip(&mask.a) {
        let g = c * v;
        *p = [g, g, g, 1.0];
    }
    Ok(t)
}

fn compute_bitmap(id: u32, n: &Node, ctx: &Ctx) -> Result<Tex, Error> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let file = n.strs.get("file").ok_or_else(|| Error::BadParam(id, "bitmap needs file".into()))?;
    let bytes = match ctx.res.get(file) {
        Some(b) => b.clone(),
        // graphs written on Windows use backslashes; accept them on every platform
        None => ctx.ec.files.read(file).map_err(|e| Error::BadParam(id, format!("cannot read '{file}': {e}")))?,
    };
    let mut t = png::decode(&bytes).map_err(|e| Error::BadParam(id, format!("'{file}': {e}")))?;
    if f("flip_x", 0.0) > 0.5 || f("flip_y", 0.0) > 0.5 {
        let (fx, fy) = (f("flip_x", 0.0) > 0.5, f("flip_y", 0.0) > 0.5);
        let src = t.clone();
        for (y, row) in t.px.chunks_mut(t.w.max(1)).enumerate() {
            for (x, d) in row.iter_mut().enumerate() {
                let (sx, sy) = (if fx { t.w - 1 - x } else { x }, if fy { t.h - 1 - y } else { y });
                if let Some(p) = src.px.get(sy * t.w + sx) {
                    *d = *p;
                }
            }
        }
    }
    if f("gray", 0.0) > 0.5 {
        t = ops::to_gray(&t);
    }
    Ok(t)
}

fn compute_channel_shuffle(a: &Tex, ins: &BTreeMap<String, Rc<Tex>>, f: impl Fn(&str, f32) -> f32) -> Tex {
    // each output channel picks a source: 0-3 channels of `in`, 4-7 channels of `in2`, 8 zero, 9 one
    let b = ins.get("in2").map(|t| t.as_ref()).unwrap_or(a);
    let pick = |k: &str, d: f32| f(k, d).round().clamp(0.0, 9.0) as usize;
    let src = [pick("src_r", 0.0), pick("src_g", 1.0), pick("src_b", 2.0), pick("src_a", 3.0)];
    let inv = [f("inv_r", 0.0) > 0.5, f("inv_g", 0.0) > 0.5, f("inv_b", 0.0) > 0.5, f("inv_a", 0.0) > 0.5];
    let mut out = a.clone();
    for (i, p) in out.px.iter_mut().enumerate() {
        let (pa, pb) = (a.px.get(i).copied().unwrap_or_default(), b.px.get(i).copied().unwrap_or_default());
        for ((d, s), inv) in p.iter_mut().zip(src).zip(inv) {
            let v = match s {
                0..=3 => pa.get(s).copied().unwrap_or(0.0),
                4..=7 => pb.get(s - 4).copied().unwrap_or(0.0),
                8 => 0.0,
                _ => 1.0,
            };
            *d = if inv { 1.0 - v } else { v };
        }
    }
    out
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let d = b - a;
    let t = if d.abs() < 1e-9 { if x >= b { 1.0 } else { 0.0 } } else { ((x - a) / d).clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

pub const EXAMPLE: &str = r#"# Brick wall PBR material
size 2048
node 1 bricks rows=8 cols=4 mortar=0.04 bevel=0.10 lum_var=0.45 seed=3
node 2 fbm scale=8 octaves=5 seed=7
node 3 fbm scale=32 octaves=3 seed=9
node 4 blend fg=@2 bg=@1 mode=5 opacity=0.35
node 5 blend fg=@3 bg=@4 mode=1 opacity=0.06
node 6 normal in=@5 strength=3
node 7 ao in=@5 radius=6 intensity=2.5
node 8 colorize in=@5 p0=0 r0=0.10 g0=0.09 b0=0.08 p1=0.5 r1=0.55 g1=0.27 b1=0.19 p2=1 r2=0.78 g2=0.46 b2=0.34
node 9 blend fg=@7 bg=@8 mode=3 opacity=1
node 10 levels in=@3 low=0.2 high=0.9 out_low=0.55 out_high=0.95
node 11 color r=0 g=0 b=0
output basecolor 9
output height 5
output normal 6
output ambient_occlusion 7
output roughness 10
output metallic 11
expose "Brick count" 1 cols 1 12
expose "Mortar width" 1 mortar 0 0.15
expose "Normal strength" 6 strength 0 10
"#;

/// Collect the connected pattern inputs of a scatter node (`pattern`, `pattern2` .. `pattern6`).
fn scatter_patterns<'a>(ins: &'a BTreeMap<String, Rc<Tex>>, builtin: u32, fallback: u32, distribution: u32) -> scatter::Patterns<'a> {
    let mut inputs: Vec<&Tex> = Vec::new();
    for k in ["pattern", "pattern2", "pattern3", "pattern4", "pattern5", "pattern6"] {
        if let Some(t) = ins.get(k) {
            inputs.push(t.as_ref());
        }
    }
    scatter::Patterns { inputs, builtin, fallback, distribution }
}

#[cfg(test)]
mod output_size_tests {
    use super::Graph;

    #[test]
    fn an_exact_log2_size_wins_over_the_inputs_and_the_graph() {
        let g = Graph::parse("size 64\nnode 1 color r=1 g=1 b=1\nnode 2 levels_mid in=@1 osize_log2_x=3 osize_log2_y=5\noutput o 2\n").unwrap();
        let t = &g.evaluate(Some(64)).unwrap()["o"];
        assert_eq!((t.w, t.h), (8, 32));
    }

    #[test]
    fn a_computed_log2_size_can_follow_the_graph() {
        let text = "size 64\nsubgraph half\nmeta kind \"function\"\nnode 2 f_size\nnode 3 f_log2 a=@2\nnode 4 f_swizzle a=@3 mask=\"x\"\nnode 5 f_float v=1\nnode 6 f_sub a=@4 b=@5\nnode 1 f_out in=@6\nend\nnode 1 color r=1 g=1 b=1\nnode 2 levels_mid in=@1 fn.osize_log2_x=\"half\" fn.osize_log2_y=\"half\"\noutput o 2\n";
        let g = Graph::parse(text).unwrap_or_else(|e| panic!("{e}"));
        let t = &g.evaluate(Some(128)).unwrap()["o"];
        assert_eq!((t.w, t.h), (64, 64));
    }

    #[test]
    fn a_compiled_switch_keeps_values_past_one() {
        let text = "size 4\nmeta compiled \"1\"\nsubgraph three\nmeta kind \"function\"\nnode 2 f_float v=3\nnode 1 f_out in=@2\nend\nsubgraph quarter\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_sample_gray a=@2 slot=\"in\" filter=0\nnode 4 f_float v=4\nnode 5 f_div_large a=@3 b=@4\nnode 1 f_out in=@5\nend\nnode 1 pixel_processor fn=\"three\"\nnode 2 color r=0 g=0 b=0\nnode 3 blend mode=21 alpha_mode=1 bg=@2 fg=@1\nnode 4 pixel_processor fn=\"quarter\" in=@3\noutput o 4\n";
        let g = Graph::parse(text).unwrap_or_else(|e| panic!("{e}"));
        let t = &g.evaluate(Some(4)).unwrap()["o"];
        assert!((t.px[0][0] - 0.75).abs() < 1e-4, "{:?}", t.px[0]);
    }

    #[test]
    fn a_compiled_pattern_program_reads_its_input_at_the_input_size() {
        // each cell of a 4 x 4 input draws a dot where its colour says, on an 8 x 8 canvas: resampling the input to 8 x 8 first
        // would move the dots (quantize's colour histogram)
        let text = "size 8\nmeta compiled \"1\"\nsubgraph lum\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_sample_color a=@2 slot=\"in\" filter=0\nnode 4 f_swizzle a=@3 mask=\"xy\"\nnode 5 f_sub a=@4 b=@2\nnode 6 f_set a=@5 name=\"v0\"\nnode 7 f_float v=1\nnode 8 f_sequence a=@6 b=@7\nnode 1 f_out in=@8\nend\nsubgraph off\nmeta kind \"function\"\nnode 2 f_get name=\"v0\" type=3\nnode 1 f_out in=@2\nend\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant pattern=0 draws=1 fn.luminance=\"lum\" fn.offset=\"off\" size_x=0.25 size_y=0.25\nnode 2 pp_quadrant draws=0 q0=@1 q1=@1 q2=@1 q3=@1\nnode 3 pp_quadrant draws=0 q0=@2 q1=@2 q2=@2 q3=@2\nnode 4 pp_out in=@3\nend\nnode 1 bitmap file=\"t.png\" osize_log2_x=2 osize_log2_y=2\nnode 2 pattern_program program=\"p\" roughness=0 in=@1 osize_log2_x=3 osize_log2_y=3\noutput o 2\n";
        // the input: every cell points at pixel (1, 6) of the canvas, except the top left one, which points at (6, 1)
        let mut g = Graph::parse(text).unwrap_or_else(|e| panic!("{e}"));
        let mut px = vec![[1.5 / 8.0, 6.5 / 8.0, 0.0, 1.0]; 16];
        px[0] = [6.5 / 8.0, 1.5 / 8.0, 0.0, 1.0];
        g.resources.insert("t.png".into(), crate::png::encode(4, 4, &px, 16));
        let t = &g.evaluate(Some(8)).unwrap()["o"];
        let lit: Vec<usize> = (0..64).filter(|&i| t.px[i][0] > 0.5).collect();
        assert_eq!(lit, vec![8 + 6, 6 * 8 + 1], "{lit:?}");
    }
}

/// The kinds [`compute_grey`] can compute on one channel.
const GREY_KINDS: &[&str] = &[
    "pixel_processor",
    "transform_matrix",
    "blend",
    "levels_mid",
    "box_blur",
    "line_blur",
    "dir_warp",
    "grad_warp",
    "grayscale_conversion",
    "output",
    "dot",
    "pattern_program",
];

/// Node `n` computed on one channel, when all its inputs are grey and what it does to them keeps them grey bit for bit
/// (the same arithmetic on each colour channel, alpha staying exactly 1). `None`: computed in RGBA.
/// With `steps`, a kernel that can computes in the steps of the node's format directly (the format applied).
fn compute_grey(id: u32, n: &Node, ins: &BTreeMap<String, Val>, ctx: &Ctx, steps: Option<&crate::tex::Steps>) -> Result<Option<Computed>, Error> {
    #[cfg(test)]
    if grey_tests::RGBA_ONLY.with(std::cell::Cell::get) {
        return Ok(None);
    }
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let grey = |slot: &str| ins.get(slot).and_then(Val::grey_in);
    let all_grey = ins.values().all(|v| v.grey_in().is_some());
    let (s, hs) = (ctx.size, ctx.height);
    Ok(match n.kind.as_str() {
        // a function giving one number is grey whatever it reads
        "pixel_processor" => match n.strs.get("fn") {
            Some(fname) => {
                let images = IMAGE_SLOTS.iter().filter_map(|slot| ins.get(*slot).map(|v| (*slot, v.img_ref()))).collect();
                // (a function giving more than one number comes back computed in RGBA)
                Some(run_function(id, n, fname, images, ctx, false, Some(steps))?)
            }
            None => None,
        },
        "transform_matrix" if all_grey => match grey("in") {
            Some(t) => match crate::atomic::transform_grey_alpha(n, t.alpha()) {
                Some(alpha) => {
                    // (its input is read many times over, often smaller than the result: as floats)
                    let bg = crate::atomic::transform_matrix_bg(n)[0];
                    let t = t.floats();
                    // (in steps when its alpha is 1 and nothing moves the result after)
                    let steps = steps.filter(|_| alpha.to_bits() == 1.0f32.to_bits() && !rolls(n));
                    Some(match crate::atomic::transform_matrix(n, &t, [s, hs], bg, crate::tex::ToSteps(steps)) {
                        crate::tex::GreyOut::Q(t) => Computed::Stepped(t),
                        crate::tex::GreyOut::F(t, _) => Computed::Grey(after_compute(t, n, s, hs), alpha),
                    })
                }
                None => None,
            },
            None => None,
        },
        // the kinds whose alpha is worked out with the value: on (value, alpha) pairs, grey again when the alpha is one
        // of 1 or +0
        "box_blur" if all_grey => grey("in").map(|t| from_pairs(after_compute(crate::atomic::box_blur(n, &pairs(t)), n, s, hs))),
        "line_blur" if all_grey => match grey("in") {
            Some(t) => Some(from_pairs(after_compute(crate::atomic::line_blur_node(id, n, &pairs(t))?, n, s, hs))),
            None => None,
        },
        "dir_warp" | "grad_warp" => match (grey("in"), ins.get("map")) {
            (Some(t), Some(map)) => {
                let t = pairs(t);
                let out = match (map, map.grey_in()) {
                    (_, Some(m)) => warp_by(n, &t, &m, [s, hs]),
                    (Val::Rgba(m), None) => warp_by(n, &t, m.as_ref(), [s, hs]),
                    _ => return Ok(None),
                };
                Some(from_pairs(after_compute(out, n, s, hs)))
            }
            _ => None,
        },
        // a grey fx-map on a grey background (or none) draws on a grey canvas
        "pattern_program" if n.nums.get("color").copied().unwrap_or(0.0) <= 0.5 => {
            let bg = match ins.get("bg") {
                Some(v) => match v.grey_in() {
                    Some(g) => Some((g.floats().into_owned(), g.alpha())),
                    None => return Ok(None),
                },
                None => None,
            };
            let name = n.strs.get("program").cloned().unwrap_or_default();
            let Some(program) = ctx.lib.get(&name) else { return Ok(None) };
            let held = pattern_images(ins);
            let images: Vec<&Tex> = held.iter().map(Rc::as_ref).collect();
            let pctx = crate::patterns::Context { lib: ctx.lib, size: [s, hs], graph_inputs: ctx.gin, seed: ctx.so, graph: ctx.graph };
            let seed = f("seed", 0.0) as u32;
            match crate::patterns::render_grey(program, &pctx, seed, &images, bg.as_ref().map(|(t, a)| (t, *a)), crate::atomic::pattern_frame(n)) {
                Ok(Some((t, a))) => Some(Computed::Grey(after_compute(t, n, s, hs), a)),
                Err(_) if ctx.ec.is_cancelled() => return Err(Error::Cancelled),
                // (an error is the RGBA form's to report)
                Ok(None) | Err(_) => None,
            }
        }
        // passed on as it is, held: no copy
        "output" | "dot" => match ins.get("in") {
            Some(v) if v.dims() == (s, hs) && !rolls(n) => Some(Computed::Held(v.clone())),
            _ => None,
        },
        // grey from any input, its alpha kept
        "grayscale_conversion" => {
            let (wr, wg, wb, wa) = (f("r", 0.3), f("g", 0.59), f("b", 0.11), f("a", 0.0));
            let (lo, hi) = if f("limit", 0.0) > 0.5 { (0.0, 1.0) } else { (f32::MIN, f32::MAX) };
            let value = |p: [f32; 4]| ((p[0] * wr + p[1] * wg + p[2] * wb + p[3] * wa).clamp(lo, hi), p[3]);
            let Some(input) = ins.get("in") else { return Ok(None) };
            let (w, h) = input.dims();
            let steps = steps.filter(|_| (w, h) == (s, hs));
            let out = match (input, input.grey_in()) {
                (_, Some(g)) => crate::tex::fill_grey(w, h, |i| value(g.with_alpha(crate::tex::Texels::texel(&g, i))), steps),
                (Val::Rgba(t), None) => crate::tex::fill_grey(w, h, |i| value(t.px_at(i)), steps),
                _ => None,
            };
            out.map(|r| match r {
                crate::tex::GreyOut::F(t, a) => Computed::Grey(after_compute(t, n, s, hs), a),
                crate::tex::GreyOut::Q(t) => Computed::Stepped(t),
            })
        }
        "levels_mid" if all_grey => match grey("in") {
            Some(t) => crate::atomic::levels_mid_grey(n, t).map(|(t, a)| Computed::Grey(after_compute(t, n, s, hs), a)),
            None => None,
        },
        "blend" if all_grey => match (grey("fg"), grey("bg")) {
            (Some(fg), Some(bg)) => {
                let set = ops::BlendSet {
                    mode: f("mode", 0.0).max(0.0) as u32,
                    opacity: f("opacity", 1.0),
                    alpha_mode: f("alpha_mode", 1.0).max(0.0) as u32,
                    crop: [f("crop_x0", 0.0), f("crop_y0", 0.0), f("crop_x1", 1.0), f("crop_y1", 1.0)],
                };
                // (in steps when nothing moves or resamples the result after)
                let steps = steps.filter(|_| crate::tex::Texels::width(&bg) == s && crate::tex::Texels::height(&bg) == hs && !rolls(n));
                ops::blend_grey(fg, bg, grey("mask"), set, steps).map(|r| match r {
                    crate::tex::GreyOut::F(t, a) => Computed::Grey(after_compute(t, n, s, hs), a),
                    crate::tex::GreyOut::Q(t) => Computed::Stepped(t),
                })
            }
            _ => None,
        },
        _ => None,
    })
}

/// A grey image as (value, alpha) pairs.
fn pairs(g: crate::tex::GreyIn) -> crate::tex::Image<[f32; 2]> {
    use crate::tex::Texels;
    let a = g.alpha();
    let mut px = vec![[0.0f32; 2]; g.len()];
    crate::par::for_each_run_mut(&mut px, 1 << 15, |first, run| {
        for (i, d) in run.iter_mut().enumerate() {
            *d = [g.texel(first + i), a];
        }
    });
    crate::tex::Image { w: g.width(), h: g.height(), px }
}

/// (value, alpha) pairs as a result: grey when the alpha is the same 1 or +0 everywhere, else RGBA.
fn from_pairs(t: crate::tex::Image<[f32; 2]>) -> Computed {
    let a = t.px.first().map_or(1.0, |p| p[1]);
    if crate::tex::grey_alpha_ok(a) && t.px.iter().all(|p| p[1].to_bits() == a.to_bits()) {
        let mut px = vec![0.0f32; t.px.len()];
        crate::par::for_each_run_mut(&mut px, 1 << 15, |first, run| {
            for (i, d) in run.iter_mut().enumerate() {
                *d = t.px.get(first + i).map_or(0.0, |p| p[0]);
            }
        });
        return Computed::Grey(Grey { w: t.w, h: t.h, px }, a);
    }
    Computed::Rgba(crate::tex::Image { w: t.w, h: t.h, px: t.px.iter().map(|p| crate::tex::Pixel::rgba(*p)).collect() })
}

/// A `dir_warp` or `grad_warp` node on (value, alpha) pairs, moved by `map`.
fn warp_by<M: crate::tex::Texels>(n: &Node, t: &crate::tex::Image<[f32; 2]>, map: &M, size: [usize; 2]) -> crate::tex::Image<[f32; 2]> {
    if n.kind == "dir_warp" { crate::atomic::dir_warp(n, t, map) } else { crate::atomic::grad_warp(n, t, map, size) }
}

/// Whether [`after_compute`] moves the result: a tile offset with a fraction.
fn rolls(n: &Node) -> bool {
    let f = |k: &str| n.nums.get(k).copied().unwrap_or(0.0);
    let (tx, ty) = (f("tile_x"), f("tile_y"));
    tx - tx.floor() > 0.0 || ty - ty.floor() > 0.0
}

/// What [`compute`] does to every result of the kinds it computes itself: resampled to the node's size, and moved by
/// the fraction of its tile offset.
fn after_compute<P: crate::tex::Pixel>(out: crate::tex::Image<P>, n: &Node, s: usize, hs: usize) -> crate::tex::Image<P> {
    let f = |k: &str, d: f32| n.nums.get(k).copied().unwrap_or(d);
    let (tx, ty) = (f("tile_x", 0.0), f("tile_y", 0.0));
    let out = if out.w != s || out.h != hs { ops::resize_to(&out, s, hs) } else { out };
    let (fx, fy) = (tx - tx.floor(), ty - ty.floor());
    if fx > 0.0 || fy > 0.0 {
        return ops::roll(&out, fx, fy);
    }
    out
}

fn compute_gpu_pattern(id: u32, n: &Node, ins: &BTreeMap<String, Val>, ctx: &Ctx<'_>) -> Result<Option<Computed>, Error> {
    // Readback and dispatch dominate the very small pattern nodes. Full-size procedural maps use the GPU.
    if ctx.ec.accelerator.is_none() || ctx.size * ctx.height < 64 * 1024 {
        return Ok(None);
    }
    let Some(program) = n.strs.get("program").and_then(|name| ctx.lib.get(name)) else { return Ok(None) };
    let held = pattern_images(ins);
    let images: Vec<&Tex> = held.iter().map(Rc::as_ref).collect();
    let background = ins.get("bg").map(|v| (v.tex(), v.grey_in().map(|g| g.alpha())));
    let size = [ctx.size, ctx.height];
    let pctx = crate::patterns::Context { lib: ctx.lib, size, graph_inputs: ctx.gin, seed: ctx.so, graph: ctx.graph };
    let seed = n.nums.get("seed").copied().unwrap_or(0.0) as u32;
    let target = crate::patterns::render_accelerated(
        program,
        &pctx,
        seed,
        &images,
        background.as_ref().map(|(t, alpha)| (t.as_ref(), *alpha)),
        crate::atomic::pattern_frame(n),
        ctx.ec,
    );
    if ctx.ec.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if matches!(target, Ok(Some(_)))
        && let Some(profiler) = &ctx.ec.profiler
    {
        profiler.accelerated_node(id, &n.kind);
    }
    target
        .map(|t| {
            t.map(|t| match t {
                crate::patterns::RasterOutput::Grey(t, a) => Computed::Grey(after_compute(t, n, size[0], size[1]), a),
                crate::patterns::RasterOutput::Rgba(t) => Computed::Rgba(after_compute(t, n, size[0], size[1])),
            })
        })
        .map_err(|e| Error::BadParam(id, e))
}

fn compute_resident_image(
    id: u32,
    n: &Node,
    ins: &BTreeMap<String, NodeValue>,
    ctx: &Ctx<'_>,
    format: &NodeFormat,
) -> Result<Option<NodeValue>, Error> {
    if let Some(accelerator) = &ctx.ec.accelerator {
        let value = match n.kind.as_str() {
            "transform_matrix" if format.code == crate::measured::COMPILED_HALF => {
                if let Some(pass) = half_transform(n, ctx, ins.get("in")) {
                    accelerator
                        .resident_half_transform(pass, &ctx.ec.cancel)
                        .map(|value| value.map(|image| NodeValue::DeviceFloat(Arc::new(DeviceFloatValue { image, host: OnceLock::new() }))))
                } else {
                    Ok(None)
                }
            }
            "pixel_processor"
                if format.code == crate::measured::COMPILED_16_BIT && !rolls(n) && ins.values().any(|v| matches!(v, NodeValue::DeviceFloat(_))) =>
            {
                return compute_resident_textured_pixel(id, n, ins, ctx);
            }
            _ => Ok(None),
        };
        if ctx.ec.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if let Some(value) = value.map_err(|reason| Error::BadParam(id, reason))? {
            if let Some(profiler) = &ctx.ec.profiler {
                profiler.resident_node(id, &n.kind);
            }
            return Ok(Some(value));
        }
    }
    if let Some(image) = compute_resident_grey(id, n, ins, ctx, format.steps.as_ref())? {
        return Ok(Some(NodeValue::Device(Arc::new(DeviceValue { image, host: OnceLock::new() }))));
    }
    compute_resident_color(id, n, ins, ctx, format.code)
}

fn compute_resident_color(id: u32, n: &Node, ins: &BTreeMap<String, NodeValue>, ctx: &Ctx<'_>, format: u32) -> Result<Option<NodeValue>, Error> {
    let Some(accelerator) = &ctx.ec.accelerator else { return Ok(None) };
    if format != crate::measured::COMPILED_16_BIT
        || !matches!(n.kind.as_str(), "blend" | "grayscale_conversion" | "dir_warp" | "transform_matrix")
        || rolls(n)
    {
        return Ok(None);
    }
    let resident = ins.values().any(|v| matches!(v, NodeValue::Device(_) | NodeValue::DeviceColor(_)));
    let area = ctx.size * ctx.height;
    if !ctx.size.is_multiple_of(2) || area < if resident { 256 * 1024 } else { 1024 * 1024 } {
        return Ok(None);
    }
    let input = |slot| ins.get(slot).and_then(NodeValue::color_input);
    let number = |key, default| n.nums.get(key).copied().unwrap_or(default);
    let binary_alpha = |input| match input {
        ColorInput::Grey(image) => Some(image.info().3),
        ColorInput::Device(image) => image.alpha,
        ColorInput::Host(image) => {
            let alpha = image.px.first()?.get(3).copied()?;
            (crate::tex::grey_alpha_ok(alpha) && image.px.iter().all(|p| p[3].to_bits() == alpha.to_bits())).then_some(alpha)
        }
    };
    let pass = if n.kind == "grayscale_conversion" {
        let Some(source) = input("in") else { return Ok(None) };
        let weights = [number("r", 0.3), number("g", 0.59), number("b", 0.11), number("a", 0.0)];
        if !weights.into_iter().all(f32::is_finite) {
            return Ok(None);
        }
        ResidentColorPass::Grayscale { source, weights, alpha: binary_alpha(source) }
    } else if n.kind == "dir_warp" {
        let (Some(source), Some(map), Some(settings)) =
            (input("in"), ins.get("map").and_then(NodeValue::packed_grey), crate::atomic::resident_directional_warp(n))
        else {
            return Ok(None);
        };
        if matches!(source, ColorInput::Grey(_)) {
            return Ok(None);
        }
        ResidentColorPass::DirectionalWarp { source, map, settings, alpha: binary_alpha(source) }
    } else if n.kind == "transform_matrix" {
        let (Some(ColorInput::Device(source)), Some((matrix, offset))) = (input("in"), crate::atomic::resident_wrapped_color_affine(n)) else {
            return Ok(None);
        };
        if source.w != ctx.size || source.h != ctx.height || source.w != source.h || !source.w.is_power_of_two() {
            return Ok(None);
        }
        ResidentColorPass::WrappedAffine { source, matrix, offset }
    } else {
        let (Some(foreground), Some(background)) = (input("fg"), input("bg")) else { return Ok(None) };
        let mask = if n.ins.contains_key("mask") {
            let Some(mask) = input("mask") else { return Ok(None) };
            Some(mask)
        } else {
            None
        };
        if [Some(foreground), Some(background), mask].into_iter().flatten().all(|source| matches!(source, ColorInput::Grey(_))) {
            return Ok(None);
        }
        let mode = number("mode", 0.0) as u32;
        let opacity = number("opacity", 1.0);
        let alpha_mode = number("alpha_mode", 1.0) as u32;
        if !crate::accelerator::packed_blend_mode(mode) || !full_blend_frame(n) || !opacity.is_finite() {
            return Ok(None);
        }
        let alpha = if !matches!(alpha_mode, 0 | 2 | 3) && (alpha_mode != 4 || mode != 21) { binary_alpha(background) } else { None };
        ResidentColorPass::Blend { foreground, background, mask, mode, opacity, alpha_mode, alpha }
    };
    let result = accelerator.resident_color(pass, &ctx.ec.cancel);
    if ctx.ec.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let Some(result) = result.map_err(|reason| Error::BadParam(id, reason))? else { return Ok(None) };
    if let Some(profiler) = &ctx.ec.profiler {
        profiler.resident_node(id, &n.kind);
    }
    Ok(Some(match result {
        ResidentColorOutput::Grey(image) => NodeValue::Device(Arc::new(DeviceValue { image, host: OnceLock::new() })),
        ResidentColorOutput::Color(image) => NodeValue::DeviceColor(Arc::new(DeviceColorValue { image, host: OnceLock::new() })),
    }))
}

/// A packed image already at the required size and quantization can pass through an identity unchanged.
fn forward_identity(n: &Node, source: Option<&NodeValue>, size: (usize, usize), format: u32) -> Option<NodeValue> {
    use crate::measured::{COMPILED_8_BIT, COMPILED_16_BIT, COMPILED_HALF};
    let source = source?;
    if source.dims() != size || rolls(n) {
        return None;
    }
    let same_storage = match source {
        NodeValue::DeviceFloat(_) => format == COMPILED_HALF,
        NodeValue::DeviceColor(_) => format == COMPILED_16_BIT,
        value => value.packed_grey().is_some_and(|packed| matches!((packed.info().2, format), (65535.0, COMPILED_16_BIT) | (255.0, COMPILED_8_BIT))),
    };
    if matches!(n.kind.as_str(), "dot" | "output") && (format == 0 || same_storage) {
        return Some(source.clone());
    }
    if n.kind != "transform_matrix" || !same_storage || !size.0.is_power_of_two() || !size.1.is_power_of_two() {
        return None;
    }
    let value = |key, default| n.nums.get(key).copied().unwrap_or(default);
    let mip = value("mip", 31.0).round() as u32;
    (matches!(mip, 0 | 30..)
        && value("offset_x", 0.0) == 0.0
        && value("offset_y", 0.0) == 0.0
        && [value("m00", 1.0), value("m01", 0.0), value("m10", 0.0), value("m11", 1.0)] == [1.0, 0.0, 0.0, 1.0])
    .then(|| source.clone())
}

/// A compiled switch at either endpoint holds exactly the selected input's steps.
fn forward_switch(g: &Graph, n: &Node, ins: &BTreeMap<String, NodeValue>, ctx: &Ctx<'_>, format: u32) -> Option<NodeValue> {
    use crate::measured::{COMPILED_8_BIT, COMPILED_16_BIT, STEPS_8, STEPS_16};
    if n.kind != "blend" || rolls(n) || n.ins.contains_key("mask") || !full_blend_frame(n) || n.nums.get("mode") != Some(&21.0) {
        return None;
    }
    let div = match format {
        COMPILED_8_BIT => STEPS_8 as f32,
        COMPILED_16_BIT => STEPS_16 as f32,
        _ => return None,
    };
    let opacity = n.nums.get("opacity").copied().unwrap_or(1.0);
    if !matches!(opacity, 0.0 | 1.0) {
        return None;
    }
    let foreground = ins.get("fg")?;
    let background = ins.get("bg")?;
    let compiled_steps = |slot, image: &NodeValue| match image.packed_grey() {
        Some(input) => input.info().2 == div,
        // A host color result has the quantization of its producer.
        None => n
            .ins
            .get(slot)
            .and_then(|id| ctx.instance.and_then(|i| i.nodes.get(id)).or_else(|| g.nodes.get(id)))
            .is_some_and(|source| !source.strs.contains_key("fn.ofmt") && node_format(g, source) == format),
    };
    if !compiled_steps("fg", foreground) || !compiled_steps("bg", background) {
        return None;
    }
    let alpha_mode = n.nums.get("alpha_mode").copied().unwrap_or(1.0) as u32;
    let same_alpha = foreground.packed_grey().zip(background.packed_grey()).is_some_and(|(fg, bg)| fg.info().3 == bg.info().3);
    if opacity == 0.0 {
        Some(background.clone())
    } else if same_alpha || matches!(alpha_mode, 0 | 2..=4) {
        Some(foreground.clone())
    } else {
        None
    }
}

fn node_format(g: &Graph, n: &Node) -> u32 {
    match n.nums.get("ofmt").map(|v| v.round() as u32) {
        Some(v @ (1..=3 | 5..=7)) => v,
        Some(4) => 0,
        _ => g.default_format(),
    }
}

fn full_blend_frame(n: &Node) -> bool {
    let coord = |name, default| n.nums.get(name).copied().unwrap_or(default);
    [coord("crop_x0", 0.0), coord("crop_y0", 0.0), coord("crop_x1", 1.0), coord("crop_y1", 1.0)] == [0.0, 0.0, 1.0, 1.0]
}

fn half_transform<'a>(n: &Node, ctx: &Ctx<'_>, source: Option<&'a NodeValue>) -> Option<HalfTransform<'a>> {
    let source = source?;
    let value = |key, default| n.nums.get(key).copied().unwrap_or(default);
    let (w, h) = source.dims();
    if w != h
        || ctx.size != ctx.height
        || !w.is_power_of_two()
        || !ctx.size.is_power_of_two()
        || rolls(n)
        || value("tile_x", 1.0) != 1.0
        || value("tile_y", 1.0) != 1.0
        || value("filter", 1.0) != 1.0
        || value("offset_x", 0.0) != 0.0
        || value("offset_y", 0.0) != 0.0
        || crate::atomic::transform_matrix_bg(n) != [0.0; 4]
        || [value("m00", 1.0), value("m01", 0.0), value("m10", 0.0), value("m11", 1.0)] != [1.0, 0.0, 0.0, 1.0]
    {
        return None;
    }
    let mip = value("mip", 31.0).round() as u32;
    // Automatic level 30 skips the successive doubling that level 31 applies when enlarging.
    if mip >= 30 && mip != 31 && w < ctx.size {
        return None;
    }
    let source = match source {
        NodeValue::DeviceFloat(value) => GreyTransformInput::Float(&value.image),
        value => {
            let source = value.packed_grey()?;
            let (_, _, div, alpha) = source.info();
            if div != 65535.0 || alpha != 1.0 || matches!(source, GreyInput::Host(_)) && w.max(ctx.size) < 1024 {
                return None;
            }
            GreyTransformInput::Quantized(source)
        }
    };
    Some(HalfTransform { source, side: ctx.size, mip: (mip < 30).then_some(mip) })
}

fn compute_resident_textured_pixel(id: u32, n: &Node, ins: &BTreeMap<String, NodeValue>, ctx: &Ctx<'_>) -> Result<Option<NodeValue>, Error> {
    let (Some(accelerator), Some(name)) = (&ctx.ec.accelerator, n.strs.get("fn")) else { return Ok(None) };
    let Some(images) = IMAGE_SLOTS
        .iter()
        .filter_map(|&slot| {
            ins.get(slot).map(|value| match value {
                NodeValue::Host(image) => Some((slot, PixelImageInput::Host(image.img_ref()))),
                NodeValue::DeviceFloat(value) => Some((slot, PixelImageInput::Float(&value.image))),
                NodeValue::Device(_) | NodeValue::DeviceColor(_) => None,
            })
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(None);
    };
    let prog = ctx.program(name).map_err(|e| Error::BadParam(id, e))?;
    let env = pixel_function_env(n, ctx, &prog);
    let Some(shader) = prog.textured_pixel_shader(&env, &images) else { return Ok(None) };
    let sources = images.iter().map(|&(_, image)| image).collect::<Vec<_>>();
    let output = accelerator.resident_textured_pixel(shader, &sources, [ctx.size, ctx.height], &ctx.ec.cancel);
    if ctx.ec.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let Some(image) = output.map_err(|reason| Error::BadParam(id, reason))? else { return Ok(None) };
    if let Some(profiler) = &ctx.ec.profiler {
        profiler.resident_node(id, &n.kind);
    }
    Ok(Some(NodeValue::Device(Arc::new(DeviceValue { image, host: OnceLock::new() }))))
}

fn colorize(n: &Node, input: &Tex) -> Option<Tex> {
    let f = |key, default| n.nums.get(key).copied().unwrap_or(default);
    let mut stops = Vec::new();
    for i in 0..512 {
        if let Some(position) = n.nums.get(&format!("p{i}")) {
            let channel = |name| n.nums.get(&format!("{name}{i}")).copied().unwrap_or(0.0);
            let alpha = n.nums.get(&format!("a{i}")).copied().unwrap_or(1.0);
            stops.push((*position, [channel("r"), channel("g"), channel("b")], alpha));
        }
    }
    if stops.is_empty() {
        return None;
    }
    let interpolation = f("interp", 0.0) as u32;
    let smoothness = f("smoothness", 1.0);
    let use_alpha = f("use_alpha", 0.0) > 0.5;
    if f("table", 0.0) > 0.5 {
        let ramp = Tex { w: 256, h: 1, px: (0..256).map(|i| [i as f32 / 255.0, i as f32 / 255.0, i as f32 / 255.0, 1.0]).collect() };
        let table = ops::colorize_ex(&ramp, &stops, interpolation, smoothness, false, true);
        Some(ops::colorize_table(input, &table, use_alpha))
    } else {
        Some(ops::colorize_ex(input, &stops, interpolation, smoothness, f("addressing", 0.0) > 0.5, use_alpha))
    }
}

/// Every possible packed input has one gradient result. Use the reference gradient
/// evaluator to build the exact curve, provided the result fits packed grayscale.
fn colorize_curve(n: &Node, div: f32, alpha: f32, steps: &crate::tex::Steps) -> Option<(Vec<u16>, f32)> {
    if !matches!(div, 255.0 | 65535.0) {
        return None;
    }
    let px = (0..=div as u16)
        .map(|i| {
            let level = f32::from(i) / div;
            [level, level, level, alpha]
        })
        .collect::<Vec<_>>();
    let rendered = colorize(n, &Tex { w: px.len(), h: 1, px })?;
    let &[_, _, _, alpha] = rendered.px.first()?;
    if !crate::tex::grey_alpha_ok(alpha) {
        return None;
    }
    let alpha_bits = alpha.to_bits();
    let curve = rendered
        .px
        .into_iter()
        .map(|[r, g, b, a]| {
            if r.to_bits() != g.to_bits() || r.to_bits() != b.to_bits() || a.to_bits() != alpha_bits {
                return None;
            }
            steps.step(r)
        })
        .collect::<Option<_>>()?;
    Some((curve, alpha))
}

fn compute_resident_grey(
    id: u32,
    n: &Node,
    ins: &BTreeMap<String, NodeValue>,
    ctx: &Ctx<'_>,
    steps: Option<&crate::tex::Steps>,
) -> Result<Option<ResidentGrey16>, Error> {
    let Some(accelerator) = &ctx.ec.accelerator else { return Ok(None) };
    let result = match n.kind.as_str() {
        "transform_matrix" | "blend" => {
            if let Some(pass) = grey_pass(n, ctx, steps, |slot| ins.get(slot).and_then(NodeValue::packed_grey)) {
                accelerator.resident_grey(pass, &ctx.ec.cancel)
            } else if n.kind == "transform_matrix"
                && let Some(steps) = steps.filter(|s| matches!(s.fmt, crate::measured::COMPILED_8_BIT | crate::measured::COMPILED_16_BIT))
                && !rolls(n)
                && let Some(source) = ins.get("in").and_then(NodeValue::packed_grey)
                && let Some(filter) = crate::atomic::resident_filter(n, [ctx.size, ctx.height], steps.div, source, None)
                && (matches!(source, GreyInput::Device(_)) || ctx.size * ctx.height >= 64 * 1024 || {
                    let (w, h, ..) = source.info();
                    // Reducing a large host image builds its pyramid even for a very small output.
                    w * h >= 1024 * 1024
                        && (w > ctx.size || h > ctx.height)
                        && matches!(filter.operation, crate::accelerator::FilterOperation::Affine { mip_side: Some(_), .. })
                })
            {
                accelerator.resident_filter(filter, &ctx.ec.cancel)
            } else {
                return Ok(None);
            }
        }
        "levels_mid" | "colorize" if !rolls(n) => {
            let Some((source, steps)) = ins.get("in").and_then(NodeValue::packed_grey).zip(steps) else { return Ok(None) };
            let (_, _, div, alpha) = source.info();
            // A smaller device image can keep the whole chain resident. Host
            // images still need to amortize both the curve and the pixel upload.
            let minimum_pixels = if matches!(source, GreyInput::Device(_)) { 512 * 512 } else { 1024 * 1024 };
            if ctx.size * ctx.height < minimum_pixels {
                return Ok(None);
            }
            let curve = match n.kind.as_str() {
                "levels_mid" => crate::atomic::levels_mid_curve(n, div, alpha, steps),
                _ => colorize_curve(n, div, alpha, steps),
            };
            let Some((curve, alpha)) = curve else { return Ok(None) };
            accelerator.resident_grey(ResidentGreyPass::Levels { source, curve, div: steps.div, alpha }, &ctx.ec.cancel)
        }
        "dir_warp" | "grad_warp" | "box_blur" | "line_blur" | "laplacian_sharpen" if !rolls(n) && ctx.size * ctx.height >= 64 * 1024 => {
            let Some(steps) = steps.filter(|s| matches!(s.fmt, crate::measured::COMPILED_8_BIT | crate::measured::COMPILED_16_BIT)) else {
                return Ok(None);
            };
            let Some(source) = ins.get("in").and_then(NodeValue::packed_grey) else { return Ok(None) };
            let map = ins.get("map").and_then(NodeValue::packed_grey);
            let Some(filter) = crate::atomic::resident_filter(n, [ctx.size, ctx.height], steps.div, source, map) else { return Ok(None) };
            accelerator.resident_filter(filter, &ctx.ec.cancel)
        }
        "pixel_processor"
            if steps.is_some_and(|s| s.fmt == crate::measured::COMPILED_16_BIT)
                && !rolls(n)
                && ins.values().any(|value| matches!(value, NodeValue::Device(_))) =>
        {
            let Some(name) = n.strs.get("fn") else { return Ok(None) };
            let Some(images) = IMAGE_SLOTS
                .iter()
                .filter_map(|&slot| ins.get(slot).map(|value| value.packed_grey().map(|input| (slot, input))))
                .collect::<Option<Vec<_>>>()
            else {
                return Ok(None);
            };
            let prog = ctx.program(name).map_err(|e| Error::BadParam(id, e))?;
            let env = pixel_function_env(n, ctx, &prog);
            let Some(shader) = prog.packed_pixel_shader(&env, &images) else { return Ok(None) };
            let sources = images.iter().map(|&(_, input)| input).collect::<Vec<_>>();
            accelerator.resident_pixel(shader, &sources, [ctx.size, ctx.height], &ctx.ec.cancel)
        }
        _ => return Ok(None),
    };
    if ctx.ec.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if result.as_ref().is_ok_and(Option::is_some)
        && let Some(profiler) = &ctx.ec.profiler
    {
        profiler.resident_node(id, &n.kind);
    }
    result.map_err(|reason| Error::BadParam(id, reason))
}

/// The imported 16-bit grayscale pyramids frequently enlarge and blend whole 4K/8K images.
fn grey_pass<'a>(
    n: &Node,
    ctx: &Ctx<'_>,
    steps: Option<&crate::tex::Steps>,
    input: impl Fn(&str) -> Option<GreyInput<'a>>,
) -> Option<ResidentGreyPass<'a>> {
    use crate::measured::{COMPILED_16_BIT, STEPS_8, STEPS_16};

    if ctx.size != ctx.height || !steps.is_some_and(|s| s.fmt == COMPILED_16_BIT) || rolls(n) {
        return None;
    }
    let value = |key, default| n.nums.get(key).copied().unwrap_or(default);
    let source = |slot, accepts_8_bit| {
        let image = input(slot)?;
        let (w, h, div, alpha) = image.info();
        let packed = div == STEPS_16 as f32 || accepts_8_bit && div == STEPS_8 as f32;
        (w == h && w.is_power_of_two() && packed && crate::tex::grey_alpha_ok(alpha)).then_some((image, w))
    };
    if n.kind == "transform_matrix" {
        let (source, width) = source("in", true)?;
        if width < 2
            || !ctx.size.is_power_of_two()
            || value("tile_x", 1.0) != 1.0
            || value("tile_y", 1.0) != 1.0
            || value("filter", 1.0) <= 0.5
            || value("mip", 31.0).round() != 31.0
            || value("offset_x", 0.0) != 0.0
            || value("offset_y", 0.0) != 0.0
            || crate::atomic::transform_matrix_bg(n) != [0.0; 4]
            || [value("m00", 1.0), value("m01", 0.0), value("m10", 0.0), value("m11", 1.0)] != [1.0, 0.0, 0.0, 1.0]
        {
            return None;
        }
        let resident = matches!(source, GreyInput::Device(_));
        if (ctx.size >= 4096 || resident) && width < ctx.size {
            Some(ResidentGreyPass::Enlarge { source, side: ctx.size })
        } else if source.info().2 == STEPS_16 as f32 && (resident || width >= 2048) && (2..width).contains(&ctx.size) {
            Some(ResidentGreyPass::Reduce { source, side: ctx.size })
        } else {
            None
        }
    } else {
        let (foreground, fg_width) = source("fg", false)?;
        let (background, bg_width) = source("bg", false)?;
        let (mode, opacity) = (value("mode", 0.0) as u32, value("opacity", 1.0));
        let mask = if n.ins.contains_key("mask") {
            let (mask, mask_width) = source("mask", false)?;
            if mask_width != ctx.size {
                return None;
            }
            Some(mask)
        } else {
            None
        };
        let resident = [Some(foreground), Some(background), mask].into_iter().flatten().any(|source| matches!(source, GreyInput::Device(_)));
        if (!resident && ctx.size < 2048)
            || !crate::accelerator::packed_blend_mode(mode)
            || !opacity.is_finite()
            || fg_width != ctx.size
            || bg_width != ctx.size
            || !full_blend_frame(n)
        {
            return None;
        }
        let alpha_mode = value("alpha_mode", 1.0) as u32;
        let (_, _, _, fa) = foreground.info();
        let (_, _, _, ba) = background.info();
        // Keep varying or non-binary alpha in the reference path. A binary foreground can
        // fold its alpha into the opacity without changing masked weights.
        let alpha = match alpha_mode {
            0 | 2..=4 if mode == 21 && fa == ba => ba,
            0 | 2..=4 if mode == 21 && mask.is_none() => ba + (fa - ba) * opacity,
            0 | 2..=4 if mode == 21 => return None,
            0 if fa == ba => ba,
            0 if mask.is_none() => ba + (fa - ba) * opacity,
            0 => return None,
            3 if mode == 0 => return None, // Premultiplied copy blends color separately.
            2 | 3 if fa == 0.0 || ba == 1.0 && (0.0..=1.0).contains(&opacity) => ba,
            2 | 3 if ba == 1.0 => return None,
            2 | 3 if mask.is_none() => fa * opacity,
            2 | 3 => return None,
            _ => ba,
        };
        if !crate::tex::grey_alpha_ok(alpha) {
            return None;
        }
        let opacity = if matches!(alpha_mode, 0 | 2..=4) && mode != 21 { opacity * fa } else { opacity };
        Some(ResidentGreyPass::Blend { foreground, background, mask, mode, opacity, alpha })
    }
}

fn compute_gpu_grey_pass(
    id: u32,
    n: &Node,
    ins: &BTreeMap<String, Val>,
    ctx: &Ctx<'_>,
    steps: Option<&crate::tex::Steps>,
) -> Result<Option<Computed>, Error> {
    use crate::accelerator::GreyPass;
    let Some(accelerator) = &ctx.ec.accelerator else { return Ok(None) };
    let pass = match grey_pass(n, ctx, steps, |slot| match ins.get(slot) {
        Some(Val::Grey16(image)) => Some(GreyInput::Host(image)),
        _ => None,
    }) {
        Some(ResidentGreyPass::Enlarge { source: GreyInput::Host(source), side }) => GreyPass::Enlarge { source, side },
        Some(ResidentGreyPass::Blend {
            foreground: GreyInput::Host(foreground),
            background: GreyInput::Host(background),
            mask: None,
            mode: mode @ (3 | 6 | 32..=34),
            opacity,
            alpha,
        }) if alpha == background.alpha => GreyPass::Blend { foreground, background, mode, opacity },
        _ => return Ok(None),
    };
    let result = accelerator.grey_pass(pass, &ctx.ec.cancel);
    if ctx.ec.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if result.as_ref().is_ok_and(Option::is_some)
        && let Some(profiler) = &ctx.ec.profiler
    {
        profiler.accelerated_node(id, &n.kind);
    }
    result.map(|out| out.map(Computed::Stepped)).map_err(|message| Error::BadParam(id, message))
}

/// Pattern image slots hold RGBA images by position. Missing inputs leave gaps before later indices.
fn pattern_images(ins: &BTreeMap<String, Val>) -> Vec<Rc<Tex>> {
    let mut images = Vec::new();
    for (index, slot) in IMAGE_SLOTS.iter().enumerate() {
        if let Some(image) = ins.get(*slot) {
            let image = image.tex();
            images.resize(index, image.clone());
            images.push(image);
        }
    }
    images
}

/// The grey kernels against the RGBA ones: every graph here rendered both ways must give the same bits.
#[cfg(test)]
mod grey_tests {
    use super::*;

    thread_local! {
        /// every node computed in RGBA (the grey inputs expanded), as before grey kernels
        pub(super) static RGBA_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        /// nodes computed grey
        pub(super) static GREY_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// A sample of a sweep of `total` cases: all of them when `craft_testkit::hostile::divisor()` is 1 (nightly), at the
    /// gate every `per_div * divisor - 1`-th (an odd stride, so every axis of a nested loop varies).
    struct Sweep {
        stride: usize,
        total: usize,
        seen: usize,
        visited: usize,
    }

    impl Sweep {
        fn new(total: usize, per_div: usize) -> Sweep {
            #[cfg(not(target_arch = "wasm32"))]
            let div = craft_testkit::hostile::divisor();
            #[cfg(target_arch = "wasm32")]
            let div = 1;
            Sweep { stride: if div == 1 { 1 } else { per_div * div - 1 }, total, seen: 0, visited: 0 }
        }

        /// Whether the next case of the sweep is in the sample.
        fn take(&mut self) -> bool {
            let take = self.seen.is_multiple_of(self.stride);
            self.seen += 1;
            self.visited += usize::from(take);
            take
        }

        /// What `full` grey runs over all the cases comes to over the cases visited.
        fn share(&self, full: usize) -> usize {
            full * self.visited / self.total
        }
    }

    fn bits(outs: &BTreeMap<String, Tex>) -> Vec<(String, Vec<u32>)> {
        outs.iter().map(|(k, t)| (k.clone(), t.px.iter().flat_map(|p| p.map(f32::to_bits)).collect())).collect()
    }

    /// `body` (nodes 10 and up, reading the grey sources 1 to 4) rendered grey and RGBA at `size`: the same bits. Then
    /// again with sources 1 and 2 held in 16- and 8-bit steps.
    fn same(body: &str, size: usize, compiled: bool) {
        same_once(body, size, compiled);
        same_once(&body.replace("@1", "@5").replace("@2", "@6"), size, compiled);
        // grey with an alpha of 0, in floats and in steps
        same_once(&body.replace("@1", "@7").replace("@2", "@8"), size, compiled);
        same_once(&body.replace("@2", "@7"), size, compiled);
    }

    fn same_once(body: &str, size: usize, compiled: bool) {
        let text = format!(
            "format 3\nsize 64\n{}node 1 perlin scale=4 seed=3\nnode 2 fbm scale=6 seed=5\nnode 3 color gray=1 v=0.25\nnode 4 white_noise seed=9\n\
             node 5 perlin scale=4 seed=3 ofmt=6\nnode 6 fbm scale=6 seed=5 ofmt=5\n\
             node 7 levels_mid in=@1 out_low=1 out_high=0\nnode 8 levels_mid in=@6 out_low=1 out_high=0 ofmt=6\n{body}",
            if compiled { "meta compiled \"1\"\n" } else { "" }
        );
        let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        RGBA_ONLY.with(|c| c.set(true));
        let want = g.evaluate(Some(size));
        RGBA_ONLY.with(|c| c.set(false));
        let got = g.evaluate(Some(size));
        match (want, got) {
            (Ok(w), Ok(g)) => assert!(bits(&w) == bits(&g), "grey and RGBA differ:\n{text}"),
            (w, g) => panic!("{:?} {:?}\n{text}", w.err(), g.err()),
        }
    }

    /// Runs `f`, and how many nodes it computed grey.
    fn greys(f: impl FnOnce()) -> usize {
        GREY_RUNS.with(|c| c.set(0));
        f();
        GREY_RUNS.with(std::cell::Cell::get)
    }

    /// A grey result is formatted as one channel, its alpha left as it is: every format keeps an alpha of 1 or +0.
    #[test]
    fn formats_keep_an_opaque_alpha() {
        for fmt in [1, 2, 3, 5, 6, 7] {
            for read_rounding in [false, true] {
                for a in [1.0f32, 0.0] {
                    let mut t = Tex::solid(2, 2, [0.3, 0.3, 0.3, a]);
                    finish_format(&mut t, fmt, read_rounding);
                    assert!(t.px.iter().all(|p| p[3].to_bits() == a.to_bits()), "format {fmt}");
                }
            }
        }
    }

    /// Grey images of alpha 0 (an inverted levels inverts the alpha too) stay grey through blends and transforms.
    #[test]
    fn clear_grey_computes_grey() {
        let n = greys(|| {
            same_once("node 10 blend fg=@7 bg=@8 mode=1\nnode 11 transform_matrix in=@10 m00=2 m11=2\noutput out 11\n", 16, true);
        });
        // (the two levels making the sources, the blend and the transform)
        assert_eq!(n, 4);
    }

    #[test]
    fn transform_matrix_grey_is_rgba() {
        let mats = [
            "m00=1 m01=0 m10=0 m11=1",
            "m00=0.5 m01=0.1 m10=-0.2 m11=0.7",
            "m00=3 m01=0 m10=0 m11=3",
            "m00=0 m01=1 m10=1 m11=0",
            "m00=-1 m01=0 m10=0 m11=1",
        ];
        let bgs = ["", "bg_r=0.5 bg_g=0.5 bg_b=0.5 bg_a=1", "bg_r=0.2 bg_g=0.3 bg_b=0.2 bg_a=1", "bg_a=0.5"];
        let mut sweep = Sweep::new(2 * mats.len() * bgs.len() * 3 * 4 * 4, 16);
        let n = greys(|| {
            for compiled in [false, true] {
                for m in mats {
                    for bg in bgs {
                        for tiles in ["tile_x=1 tile_y=1", "tile_x=0 tile_y=1", "tile_x=0 tile_y=0"] {
                            for filt in ["filter=1 mip=31", "filter=0", "filter=1 mip=2", "filter=1 mip=30"] {
                                for (osz, fmt) in [
                                    ("", ""),
                                    (" osize_log2_x=7 osize_log2_y=7", " ofmt=6"),
                                    (" osize_log2_x=4 osize_log2_y=5", " ofmt=5"),
                                    (" osize_log2_x=6 osize_log2_y=6", " ofmt=6 offset_x=0.13 offset_y=-0.3"),
                                ] {
                                    if !sweep.take() {
                                        continue;
                                    }
                                    let body = format!("node 10 transform_matrix in=@2 {m} {bg} {tiles} {filt}{osz}{fmt}\noutput out 10\n");
                                    same(&body, 32, compiled);
                                }
                            }
                        }
                    }
                }
            }
        });
        assert!(n > sweep.share(300), "{n} grey runs in {} of {} cases", sweep.visited, sweep.total);
    }

    #[test]
    fn levels_mid_grey_is_rgba() {
        let n = greys(|| {
            for compiled in [false, true] {
                for set in [
                    "low=0.2 high=0.9 mid=0.3",
                    "out_low=1 out_high=0",
                    "low=0.1 out_low=-0.2 out_high=1.3",
                    "low_r=0.1 low_g=0.1 low_b=0.1 out_low_a=0.5",
                    "low_r=0.2 high=0.7",
                    "mid=0.8 out_high_a=0",
                ] {
                    for fmt in ["", " ofmt=5", " ofmt=6", " ofmt=4"] {
                        same(&format!("node 10 levels_mid in=@2 {set}{fmt}\noutput out 10\n"), 20, compiled);
                    }
                }
            }
        });
        assert!(n > 100, "{n} grey runs");
    }

    #[test]
    fn blurs_and_warps_grey_are_rgba() {
        let n = greys(|| {
            for compiled in [false, true] {
                for fmt in ["", " ofmt=5", " ofmt=6", " ofmt=7"] {
                    for set in ["intensity=0.3", "intensity=2.5 tile_x=0", "intensity=9 aspect_x=0.5 tile_y=0"] {
                        same(&format!("node 10 box_blur in=@2 {set}{fmt}\noutput out 10\n"), 32, compiled);
                    }
                    for set in [
                        "intensity=0.3 angle=0",
                        "intensity=3 angle=0.25",
                        "intensity=0.4 angle=0.1",
                        "intensity=6 angle=0.37 tile_x=0",
                        "intensity=12 angle=0.6",
                    ] {
                        same(&format!("node 10 line_blur in=@2 {set}{fmt}\noutput out 10\n"), 32, compiled);
                    }
                    for (kind, set) in [
                        ("dir_warp", "intensity=4 angle=0.3"),
                        ("dir_warp", "intensity=9 tile_x=0 map_offset=0.2"),
                        ("grad_warp", "intensity=2"),
                        ("grad_warp", "intensity=7 tile_y=0 aspect_x=2"),
                    ] {
                        for map in ["@1", "@4", "@5"] {
                            same(&format!("node 10 {kind} in=@2 map={map} {set}{fmt}\noutput out 10\n"), 32, compiled);
                        }
                    }
                }
            }
        });
        assert!(n > 300, "{n} grey runs");
    }

    #[test]
    fn grayscale_conversion_grey_is_rgba() {
        let n = greys(|| {
            for set in ["", "r=0.5 g=0.2 b=0.3 a=0.1", "r=2 limit=1", "a=1"] {
                for fmt in ["", " ofmt=6", " ofmt=5", " ofmt=3"] {
                    // a colour input too
                    for input in ["@2", "@9"] {
                        same(
                            &format!("node 9 color r=0.2 g=0.5 b=0.9\nnode 10 grayscale_conversion in={input} {set}{fmt}\noutput out 10\n"),
                            24,
                            true,
                        );
                    }
                }
            }
        });
        assert!(n > 50, "{n} grey runs");
    }

    #[test]
    fn pattern_program_grey_is_rgba() {
        let progs = [
            "node 1 pp_quadrant pattern=0 draws=1 size_x=0.3 size_y=0.3 luminance=0.6\nnode 2 pp_quadrant draws=0 q0=@1 q3=@1\nnode 3 pp_out in=@2\n",
            "node 1 pp_quadrant pattern=3 draws=1 max=1 size_x=0.7 size_y=0.4\nnode 2 pp_quadrant draws=0 q0=@1 q1=@1 q2=@1\nnode 3 pp_out in=@2\n",
            "node 1 pp_quadrant pattern=0 draws=1 image=1 image_filter=1 size_x=0.5 size_y=0.5\nnode 2 pp_out in=@1\n",
        ];
        let n = greys(|| {
            for prog in progs {
                for set in [
                    "",
                    " bg=@2",
                    " background=0.3 roughness=0.4",
                    " bg=@7 opacity=0.5",
                    " background_r=0.2 background_g=0.2 background_b=0.2",
                    " tile_x=0",
                ] {
                    for fmt in ["", " ofmt=6"] {
                        same_once(
                            &format!(
                                "subgraph p\nmeta kind \"patterns\"\n{prog}end\nnode 10 pattern_program program=\"p\" in=@1{set}{fmt}\noutput out 10\n"
                            ),
                            32,
                            true,
                        );
                    }
                }
            }
        });
        assert!(n > 20, "{n} grey runs");
    }

    #[test]
    fn blend_grey_is_rgba() {
        let mut sweep = Sweep::new(2 * 26 * 5 * 10, 16);
        let n = greys(|| {
            for compiled in [false, true] {
                for mode in (0..=21).chain(32..=35) {
                    for am in 0..=4 {
                        for (op, extra) in [
                            ("1", ""),
                            ("0.37", " mask=@4"),
                            ("0.8", " crop_x0=0.2 crop_y1=0.6"),
                            ("-2", " ofmt=6"),
                            ("1e30", " mask=@3"),
                            ("0.37", " mask=@4 ofmt=5"),
                            ("1", " ofmt=6"),
                            ("0.5", " ofmt=2"),
                            ("0.6", " ofmt=1 tile_x=0.25"),
                            ("0.6", " ofmt=7"),
                        ] {
                            if !sweep.take() {
                                continue;
                            }
                            let body = format!("node 10 blend fg=@1 bg=@2 mode={mode} alpha_mode={am} opacity={op}{extra}\noutput out 10\n");
                            same(&body, 24, compiled);
                        }
                    }
                }
            }
        });
        assert!(n > sweep.share(300), "{n} grey runs in {} of {} cases", sweep.visited, sweep.total);
    }

    #[test]
    fn pixel_processor_function_grey_is_rgba() {
        let fns = [
            // one number from a grey read
            "node 1 f_pos\nnode 2 f_sample_gray a=@1 slot=\"in\" filter=1\nnode 3 f_float v=0.3\nnode 4 f_mul a=@2 b=@3\nnode 5 f_out in=@4\n",
            // a colour read: four lanes
            "node 1 f_pos\nnode 2 f_sample_color a=@1 slot=\"in\" filter=3\nnode 5 f_out in=@2\n",
            // a boolean
            "node 1 f_pos\nnode 2 f_swizzle a=@1 mask=\"x\"\nnode 3 f_float v=0.5\nnode 4 f_gt a=@2 b=@3\nnode 5 f_out in=@4\n",
        ];
        let n = greys(|| {
            for (k, f) in fns.iter().enumerate() {
                for (src, extra) in [("@2", ""), ("@1", " in2=@4"), ("@2", " ofmt=5"), ("@5", ""), ("@6", " in2=@5 ofmt=6")] {
                    let body =
                        format!("subgraph fx\nmeta kind \"function\"\n{f}end\nnode 10 pixel_processor fn=\"fx\" in={src}{extra}\noutput out 10\n");
                    same_once(&body, 24 + k, true);
                }
            }
        });
        assert!(n >= 6, "{n} grey runs");
    }
}
