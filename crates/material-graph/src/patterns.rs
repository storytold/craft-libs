// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pattern programs with computed parameters: the form compiled pattern maps take when the importer
//! decompiles them. A program (a graph with meta `kind "patterns"`) is a tree of `pp_*` nodes walked from
//! `pp_out`; parameters are numbers or function graphs (`fn.<param>="name"`) evaluated at every visit with
//! variables that persist over the whole walk, so one function can set what a later one reads.
//!
//! - `pp_iterate` (`first`, `body`): runs `first` once, then `body` `count` times; `f_index` is the iteration.
//! - `pp_switch` (`a`, `b`): runs `b` when `cond` is true (non-zero), else `a`.
//! - `pp_quadrant` (`q0`..`q3`): one cell, split in four for its children (top-left, top-right, bottom-left,
//!   bottom-right).
//!
//! The drawing rules below were measured against the reference app's engine with generated single-node graphs:
//! - `branch_offset` (uv) moves the quadrant's pattern and its children; `offset` (uv) only its pattern.
//! - `size` is relative to the cell, default 1 (the whole cell); negative flips. `rotation` is in turns, clockwise on
//!   screen, around the pattern's centre. Neither reaches the children.
//! - A pattern's brightness is `luminance` (default 1, negative subtracts) times the level of its depth: with
//!   roughness r and tree depth D, `r^depth / (1 + r + .. + r^D)`, or 1 when r is 0. Quadrants that draw nothing count
//!   towards the depth; iterations and switches don't.
//! - Instances add, or with `max` keep the larger value, on a canvas that starts at the background. Global opacity
//!   scales each instance. A grey canvas is clamped to 0..1 once, at the end; a colour canvas is clamped after every
//!   additive instance.
//! - Patterns wrap around the canvas. Edges are hard: each pixel is sampled once, at its centre.

use crate::context::Cancel;
use crate::func::{FnEnv, FnFrame, FnState, Prog, Value, compile_named};
use crate::graph::{Graph, Node, Spec};
use crate::lcg_rand;
use crate::math;
use crate::tex::{ImgRef, Tex};
use std::collections::BTreeMap;
use std::rc::Rc;

/// A guard against runaway programs only: the reference draws them all (ground_sand's grain is 81 cells of eight levels of
/// quadrants, 1.8 million and more instances; stopping at 2 million left a third of it black, mean 60 off). Measurement "Fx-maps
/// with random numbers, five shared causes" ("No instance limit is seen ... Our guard is now 20 million").
pub const MAX_INSTANCES: usize = 20_000_000;
/// Deepest nesting a program may reach (a guard, not measured; the library's deepest tree is 9).
const MAX_DEPTH: u32 = 64;
/// A pattern this many canvases wide or more is drawn only up to this width (a guard, not measured).
const MAX_WRAPS: i64 = 16;
/// The most repeats of a pattern a framed canvas draws on each side (a guard, not measured).
const MAX_REPEATS: i64 = 4096;

pub const SPECS: &[Spec] = &[
    Spec {
        kind: "pp_out",
        req: &["in"],
        opt: &[],
        doc: "start of a pattern program; fn.init, then fn.background, fn.roughness, fn.opacity, fn.frame and fn.frame_offset run first",
    },
    Spec {
        kind: "pp_iterate",
        req: &[],
        opt: &["first", "body"],
        doc: "run first once, then body count times (count or fn.count); f_index is the iteration",
    },
    Spec { kind: "pp_switch", req: &[], opt: &["a", "b"], doc: "run b when cond (or fn.cond) is true, else a" },
    Spec {
        kind: "pp_quadrant",
        req: &[],
        opt: &["q0", "q1", "q2", "q3"],
        doc: "one cell, split in four for q0..q3: draws pattern (square, disc, paraboloid, bell, gaussian, thorn, pyramid, brick, gradation, waves, half_bell, ridged_bell, crescent, capsule, cone, gradation_offset, hemisphere), or with image=1 the input image_index, with luminance, branch_offset_x/y, offset_x/y, size_x/y, rotation, variation, max (each also fn.<name>)",
    },
];

/// What a program run sees of the graph it is in.
pub struct Context<'a> {
    pub lib: &'a BTreeMap<String, Graph>,
    pub size: [usize; 2],
    pub graph_inputs: &'a BTreeMap<String, Value>,
    pub seed: u32,
    /// In a compiled graph, on a node whose size isn't the graph's: the graph's size, which `$outputsize` reads in the node's
    /// expressions, as in a pixel processor.
    pub graph: Option<[usize; 2]>,
}

/// The fx-map node's own parameters.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// The canvas before anything is drawn.
    pub background: f32,
    /// How bright deeper levels are, see the module docs.
    pub roughness: f32,
    /// Scales every instance.
    pub opacity: f32,
    /// The part of the pattern space the canvas shows: x min, x max, y min, y max. An axis with max <= min shows all of
    /// 0..1. The engine uses this to render one row or column of something that varies along one axis only.
    pub region: [f32; 4],
    /// Whether patterns wrap around the canvas, per axis.
    pub tile: [bool; 2],
    /// The box of pattern space that repeats: x min, x max, y min, y max. An axis with max <= min repeats every 1.
    pub repeat: [f32; 4],
    /// Draws in colour: luminance and images have four channels (a colour fx-map).
    pub color: bool,
    /// The canvas before anything is drawn, in colour.
    pub background_color: Option<[f32; 4]>,
}

impl Default for Frame {
    fn default() -> Self {
        Frame {
            background: 0.0,
            roughness: 1.0,
            opacity: 1.0,
            region: [0.0, 1.0, 0.0, 1.0],
            tile: [true, true],
            repeat: [0.0, 1.0, 0.0, 1.0],
            color: false,
            background_color: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Cell {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// What a pattern program draws on: RGBA, or grey (a grey fx-map on a grey background: every instance adds to or keeps
/// the larger of the three colour channels alike and leaves the alpha alone, so one channel and the alpha hold it all).
pub enum Canvas {
    Rgba(Tex),
    /// grey values and their alpha
    Grey(crate::tex::Grey, f32),
}

/// One resolved draw in the order of the original program walk.
pub struct RasterStamp {
    /// The pixel bounds (inclusive), unwrapped unless the canvas has a custom frame.
    pub bounds: [i32; 4],
    /// The repeat bounds (inclusive) for a custom frame.
    pub repeats: [i32; 4],
    pub center: [f32; 2],
    pub half_size: [f32; 2],
    pub rotation: [f32; 2],
    pub variation: f32,
    pub pattern: i32,
    pub brightness: [f32; 4],
    /// The raster input image, at the mip selected by its footprint.
    pub image: Option<u32>,
    pub nearest: bool,
    pub keep_larger: bool,
}

pub struct RasterRequest<'a> {
    pub size: [usize; 2],
    pub background: Option<&'a Tex>,
    pub clear: [f32; 4],
    pub color: bool,
    /// Write only the brightness when the output is known to be grayscale.
    pub grey_alpha: Option<f32>,
    pub framed: bool,
    pub region: [f32; 4],
    pub period: [f32; 2],
    pub stamps: &'a [RasterStamp],
    pub images: &'a [&'a Tex],
    pub cancel: &'a Cancel,
}

pub enum RasterOutput {
    Grey(crate::tex::Grey, f32),
    Rgba(Tex),
}

/// Limits the temporary backend-neutral draw list independently of the GPU's smaller spatial batches.
const MAX_RASTER_STAMPS: usize = 2_000_000;

struct Raster<'a> {
    background: Option<&'a Tex>,
    clear: [f32; 4],
    stamps: Vec<RasterStamp>,
    inputs: Vec<(&'a Tex, Option<Rc<Tex>>)>,
    indices: BTreeMap<(usize, u32), u32>,
    cancel: &'a Cancel,
    full: bool,
}

enum Target<'a> {
    Cpu(Canvas),
    Raster(Raster<'a>),
}

impl Target<'_> {
    fn set_background(&mut self, background: f32) {
        let gray = [background, background, background, 1.0];
        match self {
            Self::Cpu(canvas) => {
                let (w, h) = canvas.dims();
                *canvas = match canvas {
                    Canvas::Rgba(_) => Canvas::Rgba(Tex::solid(w, h, gray)),
                    Canvas::Grey(..) => Canvas::Grey(crate::tex::Grey::solid(w, h, background), 1.0),
                };
            }
            Self::Raster(raster) => {
                raster.background = None;
                raster.clear = gray;
            }
        }
    }

    fn full(&self) -> bool {
        matches!(self, Self::Raster(raster) if raster.full)
    }
}

impl Canvas {
    fn dims(&self) -> (usize, usize) {
        match self {
            Canvas::Rgba(t) => (t.w, t.h),
            Canvas::Grey(t, _) => (t.w, t.h),
        }
    }

    /// One instance onto pixel `i` ([`blend`]).
    #[inline]
    fn draw(&mut self, i: usize, value: [f32; 4], brightness: [f32; 4], keep_larger: bool, color: bool) {
        match self {
            Canvas::Rgba(t) => {
                if let Some(p) = t.px.get_mut(i) {
                    blend(p, value, brightness, keep_larger, color);
                }
            }
            // (grey only when the fx-map isn't in colour)
            Canvas::Grey(t, _) => {
                if let Some(p) = t.px.get_mut(i) {
                    let v = value[0] * brightness[0];
                    *p = if keep_larger { p.max(v) } else { *p + v };
                }
            }
        }
    }

    /// As an RGBA image.
    pub fn into_tex(self) -> Tex {
        match self {
            Canvas::Rgba(t) => t,
            Canvas::Grey(t, a) => t.expand(a),
        }
    }
}

struct Walk<'a> {
    g: &'a Graph,
    ctx: &'a Context<'a>,
    target: Target<'a>,
    cancel: Option<Cancel>,
    state: FnState,
    progs: BTreeMap<String, (Prog, FnFrame)>,
    seed: u32,
    /// The compiled-graph generator's state of the record being walked; none for the fx-map's own parameters.
    rng: Option<u32>,
    /// `f_pos` in the program's expressions: the centre of the cell being walked (measured).
    pos: [f64; 2],
    /// `f_cell_size` and `f_cell_depth`: the size of the cell being walked and how many quadrants deep it is (measured).
    cell: [f64; 2],
    index: i64,
    instances: usize,
    /// Brightness of each quadrant depth.
    levels: Vec<f32>,
    opacity: f32,
    images: &'a [&'a Tex],
    region: [f32; 4],
    tile: [bool; 2],
    /// How far apart the repeats of a pattern are, per axis.
    period: [f32; 2],
    color: bool,
    tree_depth: u32,
    /// reduced copies of the images, by image and level
    mips: BTreeMap<(usize, u32), Option<Rc<Tex>>>,
}

/// The distance between repeats per axis for a box of pattern space (x min, x max, y min, y max): its size, or 1 on an axis
/// with max <= min.
fn periods(b: [f32; 4]) -> [f32; 2] {
    [if b[1] > b[0] { b[1] - b[0] } else { 1.0 }, if b[3] > b[2] { b[3] - b[2] } else { 1.0 }]
}

/// Brightness per depth of a tree `tree_depth` quadrants deep.
fn depth_levels(roughness: f32, tree_depth: u32) -> Vec<f32> {
    if roughness == 0.0 {
        return vec![1.0; tree_depth as usize + 1];
    }
    let powers: Vec<f32> = (0..=tree_depth).map(|d| math::powi(roughness, d as i32)).collect();
    let sum: f32 = powers.iter().sum();
    powers.iter().map(|p| p / sum).collect()
}

/// Deepest quadrant below `id`, counting from 0 at the first quadrant.
fn tree_depth(g: &Graph, id: u32, memo: &mut BTreeMap<u32, i32>, guard: u32) -> i32 {
    if let Some(d) = memo.get(&id) {
        return *d;
    }
    let Some(node) = g.nodes.get(&id) else { return -1 };
    if guard > MAX_DEPTH {
        return -1;
    }
    let below = |slots: &[&str], memo: &mut BTreeMap<u32, i32>| -> i32 {
        slots.iter().filter_map(|s| node.ins.get(*s)).map(|c| tree_depth(g, *c, memo, guard + 1)).max().unwrap_or(-1)
    };
    let d = match node.kind.as_str() {
        "pp_quadrant" => below(&["q0", "q1", "q2", "q3"], memo).max(-1) + 1,
        _ => below(&["in", "first", "body", "a", "b"], memo),
    };
    memo.insert(id, d);
    d
}

impl<'a> Walk<'a> {
    fn stopped(&self) -> bool {
        self.target.full() || self.cancel.as_ref().is_some_and(Cancel::is_cancelled)
    }

    /// Runs a parameter's function, if it has one.
    fn function_value(&mut self, node: &Node, name: &str) -> Result<Option<Value>, String> {
        let Some(fname) = node.strs.get(&format!("fn.{name}")).filter(|s| !s.is_empty()) else { return Ok(None) };
        if !self.progs.contains_key(fname) {
            let p = compile_named(self.ctx.lib, fname)?;
            let f = p.frame();
            self.progs.insert(fname.clone(), (p, f));
        }
        let (prog, frame) = self.progs.get_mut(fname).ok_or("function missing")?;
        let mut env = FnEnv::new([self.ctx.size[0] as f64, self.ctx.size[1] as f64]);
        // measured: wood_rough's tiles take their frame from `$outputsize`, the graph's 512 where the node is 16 x 64
        if let Some([gw, gh]) = self.ctx.graph {
            env.own = Some([self.ctx.size[0] as f64, self.ctx.size[1] as f64]);
            env.size = [gw as f64, gh as f64];
        }
        env.seed = self.seed;
        env.index = self.index;
        env.cell = self.cell;
        env.graph_inputs = self.ctx.graph_inputs;
        for (slot, t) in crate::graph::IMAGE_SLOTS.iter().zip(self.images) {
            env.images.push((slot, ImgRef::Rgba(t)));
        }
        prog.bind(&env, frame);
        // every expression starts from the record's state
        self.state.lcg_state = self.rng;
        Ok(Some(prog.run_in_bound(&env, self.pos, &mut self.state, frame)?))
    }

    /// A parameter as a value: the function `fn.<name>` if set, else the numbers `<name>` or `<name>_x`/`_y`.
    fn param(&mut self, node: &Node, name: &str, lanes: usize, default: f32) -> Result<[f32; 2], String> {
        if let Some(v) = self.function_value(node, name)? {
            return Ok(match v {
                Value::Num(n) => [n.v[0] as f32, if n.lanes > 1 { n.v[1] as f32 } else { n.v[0] as f32 }],
                Value::Bool(b) => [f32::from(u8::from(b)); 2],
                Value::Str(_) => [default; 2],
            });
        }
        let get = |k: &str| node.nums.get(k).copied();
        Ok(if lanes == 1 {
            [get(name).unwrap_or(default); 2]
        } else {
            [get(&format!("{name}_x")).unwrap_or(default), get(&format!("{name}_y")).unwrap_or(default)]
        })
    }

    /// Luminance, in all four channels: an expression of a colour fx-map gives a colour; a number is a gray with alpha 1.
    fn luminance(&mut self, node: &Node) -> Result<[f32; 4], String> {
        if let Some(Value::Num(n)) = self.function_value(node, "luminance")? {
            let v = n.v.map(|x| x as f32);
            return Ok(if n.lanes >= 3 { [v[0], v[1], v[2], if n.lanes > 3 { v[3] } else { 1.0 }] } else { [v[0], v[0], v[0], 1.0] });
        }
        let v = node.nums.get("luminance").copied().unwrap_or(1.0);
        Ok([v, v, v, 1.0])
    }

    /// A record's seed word or expression, which adds to the state it was given.
    fn seed_of(&mut self, node: &Node) -> Result<u32, String> {
        Ok(match self.function_value(node, "seed")? {
            Some(Value::Num(n)) => n.v[0] as i64 as u32,
            _ => node.nums.get("seed").map_or(0, |s| *s as u32),
        })
    }

    /// The fx-map's own expressions run first: they can set variables the program reads.
    fn run_host_expressions(&mut self, node: &Node) -> Result<(), String> {
        self.function_value(node, "init")?;
        let scalar = |v: Option<Value>| match v {
            Some(Value::Num(n)) => Some(n.v[0] as f32),
            _ => None,
        };
        if let Some(bg) = scalar(self.function_value(node, "background")?) {
            self.target.set_background(bg.clamp(0.0, 1.0));
        }
        if let Some(r) = scalar(self.function_value(node, "roughness")?) {
            self.levels = depth_levels(r, self.tree_depth);
        }
        if let Some(o) = scalar(self.function_value(node, "opacity")?) {
            self.opacity = o;
        }
        if let Some(Value::Num(n)) = self.function_value(node, "frame")?
            && n.lanes == 4
        {
            self.region = [n.v[0] as f32, n.v[1] as f32, n.v[2] as f32, n.v[3] as f32];
        }
        if let Some(Value::Num(n)) = self.function_value(node, "frame_offset")?
            && n.lanes == 4
        {
            self.period = periods([n.v[0] as f32, n.v[1] as f32, n.v[2] as f32, n.v[3] as f32]);
        }
        Ok(())
    }

    fn child(&mut self, node: &Node, slot: &str, cell: Cell, depth: u32, quads: usize) -> Result<(), String> {
        if let Some(c) = node.ins.get(slot).copied() {
            self.visit(c, cell, depth + 1, quads)?;
        }
        Ok(())
    }

    fn visit(&mut self, id: u32, cell: Cell, depth: u32, quads: usize) -> Result<(), String> {
        if depth > MAX_DEPTH || self.instances >= MAX_INSTANCES || self.stopped() {
            return Ok(());
        }
        let Some(node) = self.g.nodes.get(&id) else { return Ok(()) };
        self.pos = [f64::from(cell.x), f64::from(cell.y)];
        self.cell = [f64::from(cell.w), quads as f64];
        match node.kind.as_str() {
            "pp_out" => {
                self.run_host_expressions(node)?;
                self.rng = Some(self.seed_of(node)?);
                self.child(node, "in", cell, depth, quads)?
            }
            "pp_iterate" => {
                let above = self.rng;
                let mut state = above.unwrap_or(0).wrapping_add(self.seed_of(node)?);
                self.rng = Some(state);
                let count = self.param(node, "count", 1, 1.0)?[0].round().clamp(0.0, MAX_INSTANCES as f32) as i64;
                let outer = self.index;
                self.index = 0;
                self.child(node, "first", cell, depth, quads)?;
                for i in 0..count {
                    if self.instances >= MAX_INSTANCES || self.stopped() {
                        break;
                    }
                    self.index = i;
                    state = lcg_rand::step(lcg_rand::ITERATION, state);
                    self.rng = Some(state);
                    self.child(node, "body", cell, depth, quads)?;
                }
                self.index = outer;
                self.rng = above;
            }
            "pp_switch" => {
                let above = self.rng;
                self.rng = Some(above.unwrap_or(0).wrapping_add(self.seed_of(node)?));
                let c = self.param(node, "cond", 1, 0.0)?[0];
                self.child(node, if c != 0.0 { "b" } else { "a" }, cell, depth, quads)?;
                self.rng = above;
            }
            "pp_quadrant" => {
                let above = self.rng;
                self.rng = Some(above.unwrap_or(0).wrapping_add(self.seed_of(node)?));
                // in the order the engine stores them: an expression can set what a later one reads
                let luminance = self.luminance(node)?;
                let bo = self.param(node, "branch_offset", 2, 0.0)?;
                let off = self.param(node, "offset", 2, 0.0)?;
                let size = self.param(node, "size", 2, 1.0)?;
                let rot = self.param(node, "rotation", 1, 0.0)?[0];
                let variation = self.param(node, "variation", 1, 0.0)?[0];
                // an image quadrant's input: `image_index`, or in texts before format 3 its `pattern` number
                let image_index =
                    self.param(node, "image_index", 1, node.nums.get("image_index").or_else(|| node.nums.get("pattern")).copied().unwrap_or(0.0))?[0];
                let here = Cell { x: cell.x + bo[0], y: cell.y + bo[1], ..cell };
                if node.nums.get("draws").copied().unwrap_or(0.0) > 0.5 {
                    let level = self.levels.get(quads).copied().unwrap_or(0.0);
                    let k = level * self.opacity;
                    self.draw(
                        node,
                        here,
                        off,
                        size,
                        rot,
                        variation,
                        image_index,
                        [luminance[0] * k, luminance[1] * k, luminance[2] * k, luminance[3]],
                    );
                }
                for (q, (slot, (dx, dy))) in
                    ["q0", "q1", "q2", "q3"].iter().zip([(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)]).enumerate()
                {
                    let sub = Cell { x: here.x + dx * here.w, y: here.y + dy * here.h, w: here.w * 0.5, h: here.h * 0.5 };
                    let parent = self.rng;
                    self.rng = parent.map(|s| lcg_rand::child_state(s, q, quads + 1));
                    self.child(node, slot, sub, depth, quads + 1)?;
                    self.rng = parent;
                }
                self.rng = above;
            }
            _ => {}
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(&mut self, node: &Node, cell: Cell, off: [f32; 2], size: [f32; 2], rot: f32, variation: f32, image_index: f32, brightness: [f32; 4]) {
        let pattern = node.nums.get("pattern").copied().unwrap_or(0.0).round() as i32;
        // an index past the inputs takes the last one (measured: an index expression of 2 to 5 over two inputs draws the second;
        // fabric_lace's 2 + floor(2.99 v) over three inputs drew nothing in ours, mean 114 off)
        let image_index = image_index.max(0.0).min(self.images.len().saturating_sub(1) as f32);
        // an image pattern: byte 1 is an index into the node's inputs, the image fills the pattern's box
        let image = if node.nums.get("image").copied().unwrap_or(0.0) > 0.5 {
            let Some(t) = self.images.get(image_index as usize).copied() else { return };
            Some(t)
        } else {
            None
        };
        let keep_larger = node.nums.get("max").copied().unwrap_or(0.0) > 0.5;
        self.instances += 1;
        let (cx, cy) = (cell.x + off[0], cell.y + off[1]);
        // half the pattern's box, signed: a negative size flips it
        let (hw, hh) = (size[0] * cell.w * 0.5, size[1] * cell.h * 0.5);
        let [cw, ch] = self.ctx.size.map(|v| v as i64);
        let rad = (hw * hw + hh * hh).sqrt();
        if rad <= 0.0 || !rad.is_finite() || !cx.is_finite() || !cy.is_finite() || hw == 0.0 || hh == 0.0 {
            return;
        }
        let (sn, cs) = math::sin_cos(-rot * std::f32::consts::TAU);
        if !sn.is_finite() || !cs.is_finite() {
            return;
        }
        // Bin the rotated rectangle, with a pixel of slack for the floating-point edge test.
        // A square built from the diagonal is much larger for long, thin stamps.
        let (rad_x, rad_y) = if matches!(self.target, Target::Raster(_)) {
            let norm = sn * sn + cs * cs;
            let projected = |a: f32, b: f32, pixels: i64| ((hw * a).abs() + (hh * b).abs()) / norm + 1.0 / pixels as f32;
            (projected(cs, sn, cw).min(rad), projected(sn, cs, ch).min(rad))
        } else {
            (rad, rad)
        };
        // 0: the pyramid level for the footprint, bilinear. 1: the image itself, bilinear. 2: the image itself, nearest.
        let filter = node.nums.get("image_filter").copied().unwrap_or(0.0).round() as i32;
        // instances come by the thousand and mostly share a level, so a level is made once
        let image_mip = image.and_then(|t| {
            if filter != 0 {
                return None;
            }
            let footprint = (t.w as f32 / (2.0 * hw.abs() * cw as f32)).max(t.h as f32 / (2.0 * hh.abs() * ch as f32));
            let level = mip_level(footprint);
            self.mips.entry((image_index as usize, level)).or_insert_with(|| mip_of(t, level).map(Rc::new)).clone().map(|mip| (level, mip))
        });
        let raster_image = |raster: &mut Raster<'a>| {
            image.map(|source| {
                let key = (image_index as usize, image_mip.as_ref().map_or(0, |(level, _)| *level));
                *raster.indices.entry(key).or_insert_with(|| {
                    let index = raster.inputs.len() as u32;
                    raster.inputs.push((source, image_mip.as_ref().map(|(_, mip)| mip.clone())));
                    index
                })
            })
        };
        let collect = |raster: &mut Raster<'a>, bounds: [i64; 4], repeats: [i64; 4]| {
            if bounds[0] > bounds[2] || bounds[1] > bounds[3] || repeats[0] > repeats[2] || repeats[1] > repeats[3] {
                return;
            }
            // Keep shader indexing and float pixel centres precise even for unexpectedly large offsets.
            if raster.stamps.len() >= MAX_RASTER_STAMPS || bounds.iter().any(|v| !(-4_000_000..=4_000_000).contains(v)) {
                raster.full = true;
                return;
            }
            let image = raster_image(raster);
            raster.stamps.push(RasterStamp {
                bounds: bounds.map(|v| v as i32),
                repeats: repeats.map(|v| v as i32),
                center: [cx, cy],
                half_size: [hw, hh],
                rotation: [sn, cs],
                variation,
                pattern,
                brightness,
                image,
                nearest: filter >= 2,
                keep_larger,
            });
        };
        let image = image.map(|t| image_mip.as_ref().map_or(t, |(_, mip)| mip.as_ref()));
        let color = self.color;
        let sample = |px: f32, py: f32| -> Option<[f32; 4]> {
            let (u, v) = (px - cx, py - cy);
            let (lx, ly) = ((u * cs - v * sn) / hw, (u * sn + v * cs) / hh);
            match image {
                Some(t) if in_box(lx, ly) => {
                    let (iu, iv) = ((lx + 1.0) * 0.5, (ly + 1.0) * 0.5);
                    let p = if filter >= 2 {
                        t.px_at(
                            ((iv * t.h as f32) as usize).min(t.h.saturating_sub(1)) * t.w + ((iu * t.w as f32) as usize).min(t.w.saturating_sub(1)),
                        )
                    } else {
                        crate::atomic::sample_stepped_bordered(t, iu, iv)
                    };
                    Some(if color { p } else { [p[0], p[0], p[0], 1.0] })
                }
                Some(_) => None,
                // in colour the shape is coverage only: the colour is the luminance's, laid over by shape times alpha (measured on
                // matfinish_brushed_linear's strokes: a colour scaled by the shape too came out 57 steps dark)
                None => shape(pattern, lx, ly, variation).map(|a| if color { [1.0, 1.0, 1.0, a] } else { [a, a, a, a] }),
            }
        };
        let [rx0, rx1, ry0, ry1] = self.region;
        let (rw, rh) = (rx1 - rx0, ry1 - ry0);
        let [px_, py_] = self.period;
        let framed = framed_region(self.region, self.period);
        if framed {
            // the box's copies: every repeat that reaches the part of pattern space the canvas shows
            let copies = |c: f32, rad: f32, tile: bool, lo: f32, span: f32, period: f32| {
                if !tile {
                    return 0..=0;
                }
                let (lo, hi) = if span > 0.0 { (lo, lo + span) } else { (0.0, 1.0) };
                let first = ((c - rad - hi) / period).floor() as i64;
                let last = ((c + rad - lo) / period).ceil() as i64;
                first.max(-MAX_REPEATS)..=last.min(MAX_REPEATS)
            };
            let (kx, ky) = (copies(cx, rad_x, self.tile[0], rx0, rw, px_), copies(cy, rad_y, self.tile[1], ry0, rh, py_));
            // an instance is often far smaller than the canvas: only the rows and columns its copies can reach
            let (columns, rows) = (pixels_near(cx, rad_x, kx.clone(), px_, rx0, rw, cw), pixels_near(cy, rad_y, ky.clone(), py_, ry0, rh, ch));
            if let Target::Raster(raster) = &mut self.target {
                let repeats = [*kx.start(), *ky.start(), *kx.end(), *ky.end()];
                for &(y0, y1) in &rows {
                    for &(x0, x1) in &columns {
                        collect(raster, [x0, y0, x1, y1], repeats);
                    }
                }
                return;
            }
            let Target::Cpu(canvas) = &mut self.target else { return };
            for y in rows.iter().flat_map(|r| r.0..=r.1) {
                if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                    return;
                }
                let v = (y as f32 + 0.5) / ch as f32;
                let py = if rh > 0.0 { ry0 + v * rh } else { v };
                for x in columns.iter().flat_map(|r| r.0..=r.1) {
                    let u = (x as f32 + 0.5) / cw as f32;
                    let px = if rw > 0.0 { rx0 + u * rw } else { u };
                    for j in ky.clone() {
                        for i in kx.clone() {
                            let Some(a) = sample(px + i as f32 * px_, py + j as f32 * py_) else { continue };
                            canvas.draw((y * cw + x) as usize, a, brightness, keep_larger, color);
                        }
                    }
                }
            }
            return;
        }
        let mut x0 = ((cx - rad_x) * cw as f32).floor() as i64;
        let mut x1 = ((cx + rad_x) * cw as f32).ceil() as i64;
        let mut y0 = ((cy - rad_y) * ch as f32).floor() as i64;
        let mut y1 = ((cy + rad_y) * ch as f32).ceil() as i64;
        let centre = ((cx * cw as f32) as i64, (cy * ch as f32) as i64);
        (x0, x1) = (x0.max(centre.0 - MAX_WRAPS * cw), x1.min(centre.0 + MAX_WRAPS * cw));
        (y0, y1) = (y0.max(centre.1 - MAX_WRAPS * ch), y1.min(centre.1 + MAX_WRAPS * ch));
        if !self.tile[0] {
            (x0, x1) = (x0.max(0), x1.min(cw - 1));
        }
        if !self.tile[1] {
            (y0, y1) = (y0.max(0), y1.min(ch - 1));
        }
        if let Target::Raster(raster) = &mut self.target {
            collect(raster, [x0, y0, x1, y1], [0; 4]);
            return;
        }
        let Target::Cpu(canvas) = &mut self.target else { return };
        for y in y0..=y1 {
            if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                return;
            }
            for x in x0..=x1 {
                let Some(a) = sample((x as f32 + 0.5) / cw as f32, (y as f32 + 0.5) / ch as f32) else { continue };
                let i = (y.rem_euclid(ch) * cw + x.rem_euclid(cw)) as usize;
                canvas.draw(i, a, brightness, keep_larger, color);
            }
        }
    }
}

/// The ascending, separate ranges of pixels (of `n` along an axis) where a box of radius `rad` around `c` may show, for its
/// copies shifted by `period` times each of `copies`. A pixel's position is `start + (i + 0.5) / n * span` (or without the
/// start and span when `span` isn't positive). Errs on the side of too many pixels: a pixel just outside is rejected
/// by the box test itself.
fn pixels_near(c: f32, rad: f32, copies: std::ops::RangeInclusive<i64>, period: f32, start: f32, span: f32, n: i64) -> Vec<(i64, i64)> {
    let margin = rad * 1e-3 + 1e-3;
    let index = |at: f32| if span > 0.0 { (at - start) / span * n as f32 - 0.5 } else { at * n as f32 - 0.5 };
    let mut ranges: Vec<(i64, i64)> = Vec::new();
    for k in copies {
        let (lo, hi) = (index(c - k as f32 * period - rad - margin), index(c - k as f32 * period + rad + margin));
        // not a number: no telling, so all of them; `as` saturates
        let (first, last) = if lo.is_nan() || hi.is_nan() {
            (0, n - 1)
        } else {
            ((lo.floor() as i64).saturating_sub(1).max(0), (hi.ceil() as i64).saturating_add(1).min(n - 1))
        };
        if first <= last {
            ranges.push((first, last));
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match merged.last_mut() {
            Some(m) if r.0 <= m.1 + 1 => m.1 = m.1.max(r.1),
            _ => merged.push(r),
        }
    }
    merged
}

/// One instance onto a pixel: `value` is what the pattern shows here, `brightness` what it is multiplied by. Gray: add, or keep
/// the larger value. Colour: add, or lay over by its alpha.
fn blend(p: &mut [f32; 4], value: [f32; 4], brightness: [f32; 4], keep_larger: bool, color: bool) {
    if color {
        let v: [f32; 3] = [value[0] * brightness[0], value[1] * brightness[1], value[2] * brightness[2]];
        let a = (value[3] * brightness[3]).clamp(0.0, 1.0);
        for (c, v) in p.iter_mut().zip(v) {
            *c = if keep_larger { *c + (v - *c) * a } else { (*c + v * a).clamp(0.0, 1.0) };
        }
        p[3] = if keep_larger { p[3] + (1.0 - p[3]) * a } else { (p[3] + a).clamp(0.0, 1.0) };
        return;
    }
    let v = value[0] * brightness[0];
    for c in p.iter_mut().take(3) {
        // limited to 0..1 only at the end: a later instance can take back what a pixel went over or under (measured)
        *c = if keep_larger { c.max(v) } else { *c + v };
    }
}

/// The mipmap level the engine reads when `scale` source pixels pass per output pixel: the nearest whole level, with no
/// filtering across levels (measured on transforms at 14 scales; exactly sqrt(2) rounds up). Measurement "Fx-map drawing"
/// ("Transforms (V)": `round(log2(longer column))`, no filtering across levels).
pub fn mip_level(scale: f32) -> u32 {
    if scale.is_finite() && scale > 1.0 { (math::log2(scale) + MIP_ROUND_NUDGE).round().clamp(0.0, MAX_MIP_LEVEL) as u32 } else { 0 }
}

/// Lifts a scale of exactly sqrt(2), whose log2 single precision may put just under 0.5, to the next level, as measured (the
/// size of the nudge is a guard).
const MIP_ROUND_NUDGE: f32 = 1e-4;
/// The deepest pyramid level read: an image of 4096 pixels down to one (a guard, not measured).
const MAX_MIP_LEVEL: f32 = 12.0;

/// Level `level` of the image's pyramid, each level the 2x2 mean of the one before, an axis stopping at one pixel. `None`
/// for level 0: the image itself.
pub fn mip_of<P: crate::tex::Pixel>(t: &crate::tex::Image<P>, level: u32) -> Option<crate::tex::Image<P>> {
    let (w, h) = (t.w.checked_shr(level).unwrap_or(0).max(1), t.h.checked_shr(level).unwrap_or(0).max(1));
    if level == 0 || (w, h) == (t.w, t.h) {
        return None;
    }
    // each level is the 2x2 mean of the one before, rounded half up to a 16-bit step
    let half = |t: &crate::tex::Image<P>| {
        let mut m = crate::ops::resize_to(t, (t.w / 2).max(1), (t.h / 2).max(1));
        crate::ops::round_to_16bit_read(&mut m);
        m
    };
    let mut m = half(t);
    while (m.w, m.h) != (w, h) {
        m = half(&m);
    }
    Some(m)
}

/// Whether a point in a pattern's local coordinates is in its box. A pixel centre on the line between two boxes belongs to
/// the one left of it and above it (measured: 32 x 32 cells on a 16 x 16 image show the top-left cell of each four).
fn in_box(x: f32, y: f32) -> bool {
    x > -1.0 && x <= 1.0 && y > -1.0 && y <= 1.0
}

/// Hermite step 0..1 for `t` in 0..1.
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// The built-in patterns by number, measured: "Fx-map drawing" ("Patterns (V)"), and for the gaussian "One-effect
// projects" › "Node rules" and "What nodes hold and how they read".
/// Radius of the capsule's round end, in pattern units: `1 - 20 d^2`.
const CAPSULE_RADIUS: f32 = 0.223_606_8; // 1 / sqrt(20)
/// How far the capsule's straight part reaches at variation 1, in pattern units (measured: `max(|x| - 0.7698 v, 0)`).
const CAPSULE_REACH: f32 = 0.7698;
/// The gaussian (pattern 4): `exp(-11.1 r^2) - exp(-11.1)` inside the disc (not `2^(-16 r^2)`).
const GAUSSIAN_FALLOFF: f32 = 11.1;
/// Waves (pattern 9): `(1 - r^2) cos^2((1.5 + 6 v) pi x)`.
const WAVES_BASE: f32 = 1.5;
const WAVES_PER_VARIATION: f32 = 6.0;
/// Crescent (pattern 12): `6.75 r^2 (1 - r^2)^2 F(x + 1, 2 (1 - v))`, 6.75 bringing its peak to 1.
const CRESCENT_GAIN: f32 = 6.75;

/// Pattern `kind` at local coordinates (−1..1 across its box), `None` outside it. Measured against the engine:
/// `variation` (0..1) reshapes bricks, gradations, waves, half bells, crescents, capsules and hemispheres.
pub(crate) fn shape(kind: i32, x: f32, y: f32, variation: f32) -> Option<f32> {
    use std::f32::consts::PI;
    if !in_box(x, y) {
        return None;
    }
    let v = variation.clamp(0.0, 1.0);
    let r2 = x * x + y * y;
    let r = r2.sqrt();
    let inside = f32::from(u8::from(r <= 1.0));
    let paraboloid = (1.0 - r2).max(0.0);
    let bell = math::powi(paraboloid, 4);
    // falls smoothly from 1 at 0 to 0 at `span`
    let falloff = |t: f32, span: f32| if span <= 0.0 { 0.0 } else { 1.0 - smooth(t / span) };
    Some(match kind {
        0 => 1.0,
        1 => inside,
        2 => paraboloid,
        3 => bell,
        // reaches 0 at the rim; measured on a 16-bit node at two sizes to within the export's rounding, at every radius
        4 => (math::exp(-GAUSSIAN_FALLOFF * r2) - math::exp(-GAUSSIAN_FALLOFF)).max(0.0) * inside,
        5 => math::powi((1.0 - r).max(0.0), 4),
        6 => 1.0 - x.abs().max(y.abs()),
        7 => {
            if v <= 0.0 {
                1.0
            } else {
                smooth((1.0 - x.abs()) / v) * smooth((1.0 - y.abs()) / v)
            }
        }
        8 => (x + 1.0) * 0.5 * (1.0 - v),
        // the number of waves isn't limited: variation goes past 1 (noise_messy_fibers, 16 lines)
        9 => paraboloid * math::powi(math::cos((WAVES_BASE + WAVES_PER_VARIATION * variation) * PI * x), 2),
        10 => bell * if x > 0.0 { falloff(x, 1.0 - v) } else { 1.0 },
        11 => bell * math::powi(1.0 - x.abs(), 2),
        12 => CRESCENT_GAIN * r2 * paraboloid * paraboloid * falloff(x + 1.0, 2.0 * (1.0 - v)),
        13 => {
            let d = (x.abs() - v * CAPSULE_REACH).max(0.0);
            (1.0 - (d * d + y * y) / (CAPSULE_RADIUS * CAPSULE_RADIUS)).max(0.0)
        }
        14 => (1.0 - r).max(0.0),
        15 => ((x + 1.0) * 0.5 + v).min(1.0),
        16 => ((1.0 - r2).max(0.0).sqrt() + v).min(1.0) * inside,
        _ => inside,
    })
}

/// Run `program` on a canvas of the context's size.
pub fn render(program: &Graph, ctx: &Context<'_>, seed: u32, images: &[&Tex], background: Option<&Tex>, frame: Frame) -> Result<Tex, String> {
    let [w, h] = ctx.size;
    let bg = frame.background.clamp(0.0, 1.0);
    let canvas = Canvas::Rgba(match background {
        // an image to draw on, read once per pixel as any other input of another size
        Some(t) if (t.w, t.h) == (w, h) => t.clone(),
        Some(t) => Tex::from_fn(w, h, |u, v| crate::atomic::sample_stepped(t, u, v)),
        None => Tex::solid(w, h, frame.background_color.unwrap_or([bg, bg, bg, 1.0])),
    });
    Ok(render_on(program, ctx, seed, images, canvas, frame)?.into_tex())
}

/// [`render`] of a grey fx-map (not in colour) on a grey background (`background`: an image of alpha `alpha`, or none):
/// grey, the same values. `None` where it can't be: in colour, or a background colour that isn't grey.
pub(crate) fn render_grey(
    program: &Graph,
    ctx: &Context<'_>,
    seed: u32,
    images: &[&Tex],
    background: Option<(&crate::tex::Grey, f32)>,
    frame: Frame,
) -> Result<Option<(crate::tex::Grey, f32)>, String> {
    if frame.color {
        return Ok(None);
    }
    let [w, h] = ctx.size;
    let bg = frame.background.clamp(0.0, 1.0);
    let canvas = match background {
        Some((t, a)) if (t.w, t.h) == (w, h) => Canvas::Grey(t.clone(), a),
        Some((t, a)) => Canvas::Grey(crate::tex::Grey::from_fn(w, h, |u, v| crate::atomic::sample_stepped(t, u, v)), a),
        None => match frame.background_color.unwrap_or([bg, bg, bg, 1.0]) {
            c if c[0].to_bits() == c[1].to_bits() && c[1].to_bits() == c[2].to_bits() && crate::tex::grey_alpha_ok(c[3]) => {
                Canvas::Grey(crate::tex::Grey::solid(w, h, c[0]), c[3])
            }
            _ => return Ok(None),
        },
    };
    Ok(match render_on(program, ctx, seed, images, canvas, frame)? {
        Canvas::Grey(t, a) => Some((t, a)),
        Canvas::Rgba(_) => None,
    })
}

fn render_on(program: &Graph, ctx: &Context<'_>, seed: u32, images: &[&Tex], canvas: Canvas, frame: Frame) -> Result<Canvas, String> {
    let walk = walk_program(program, ctx, seed, images, Target::Cpu(canvas), frame)?;
    let Target::Cpu(mut out) = walk.target else { return Err("pattern program has no CPU canvas".into()) };
    if walk.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
        return Err("pattern program was cancelled".into());
    }
    match &mut out {
        Canvas::Rgba(t) => {
            for p in &mut t.px {
                for c in p.iter_mut().take(3) {
                    *c = c.clamp(0.0, 1.0);
                }
            }
        }
        Canvas::Grey(t, _) => t.px.iter_mut().for_each(|c| *c = c.clamp(0.0, 1.0)),
    }
    Ok(out)
}

/// Prepare the sequential program once and let the backend rasterize the ordered draws.
pub(crate) fn render_accelerated(
    program: &Graph,
    ctx: &Context<'_>,
    seed: u32,
    images: &[&Tex],
    background: Option<(&Tex, Option<f32>)>,
    frame: Frame,
    eval: &crate::context::EvalContext,
) -> Result<Option<RasterOutput>, String> {
    let Some(accelerator) = &eval.accelerator else { return Ok(None) };
    let cancel = &eval.cancel;
    let bg = frame.background.clamp(0.0, 1.0);
    let target = Target::Raster(Raster {
        background: background.map(|(image, _)| image),
        clear: frame.background_color.unwrap_or([bg, bg, bg, 1.0]),
        stamps: Vec::new(),
        inputs: Vec::new(),
        indices: BTreeMap::new(),
        cancel,
        full: false,
    });
    let walk = walk_program(program, ctx, seed, images, target, frame)?;
    let Target::Raster(raster) = walk.target else { return Ok(None) };
    if raster.full || cancel.is_cancelled() {
        return Ok(None);
    }
    let images: Vec<&Tex> = raster.inputs.iter().map(|(src, mip)| mip.as_deref().unwrap_or(src)).collect();
    let framed = framed_region(walk.region, walk.period);
    let grey_alpha = if frame.color {
        None
    } else if raster.background.is_some() {
        background.and_then(|(_, alpha)| alpha)
    } else {
        let [r, g, b, a] = raster.clear;
        (r.to_bits() == g.to_bits() && g.to_bits() == b.to_bits() && crate::tex::grey_alpha_ok(a)).then_some(a)
    };
    accelerator.pattern(RasterRequest {
        size: ctx.size,
        background: raster.background,
        clear: raster.clear,
        color: frame.color,
        grey_alpha,
        framed,
        region: walk.region,
        period: walk.period,
        stamps: &raster.stamps,
        images: &images,
        cancel,
    })
}

fn framed_region([x0, x1, y0, y1]: [f32; 4], [x, y]: [f32; 2]) -> bool {
    let (w, h) = (x1 - x0, y1 - y0);
    (w > 0.0 && (x0 != 0.0 || w != 1.0)) || (h > 0.0 && (y0 != 0.0 || h != 1.0)) || x != 1.0 || y != 1.0
}

fn walk_program<'a>(
    program: &'a Graph,
    ctx: &'a Context<'a>,
    seed: u32,
    images: &'a [&'a Tex],
    target: Target<'a>,
    frame: Frame,
) -> Result<Walk<'a>, String> {
    let roots: Vec<u32> = program.nodes.iter().filter(|(_, n)| n.kind == "pp_out").map(|(i, _)| *i).collect();
    let [root] = roots.as_slice() else { return Err("a pattern program needs exactly one pp_out node".into()) };
    program.check_cycles().map_err(|e| e.to_string())?;
    let depth = tree_depth(program, *root, &mut BTreeMap::new(), 0).max(0) as u32;
    let cancel = match &target {
        Target::Cpu(_) => crate::context::current_cancel(),
        Target::Raster(raster) => Some(raster.cancel.clone()),
    };
    let mut walk = Walk {
        g: program,
        ctx,
        target,
        cancel,
        state: FnState::default(),
        progs: BTreeMap::new(),
        mips: BTreeMap::new(),
        seed: seed.wrapping_add(ctx.seed),
        rng: None,
        pos: [0.5, 0.5],
        cell: [1.0, 0.0],
        index: 0,
        instances: 0,
        levels: depth_levels(frame.roughness, depth),
        opacity: frame.opacity,
        images,
        region: frame.region,
        tile: frame.tile,
        period: periods(frame.repeat),
        color: frame.color,
        tree_depth: depth,
    };
    walk.visit(*root, Cell { x: 0.5, y: 0.5, w: 1.0, h: 1.0 }, 0, 0)?;
    Ok(walk)
}

#[cfg(test)]
mod tests {
    use super::pixels_near;

    #[test]
    fn pixels_near_covers_every_pixel_a_box_reaches() {
        let mut seed = 0x2545F4914F6CDD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / (1u64 << 24) as f32
        };
        for _ in 0..3000 {
            let (c, rad) = (next() * 3.0 - 1.0, math::powi(next(), 4) * 2.0 + 1e-5);
            let (start, span) = (next() * 2.0 - 1.0, if next() < 0.2 { 0.0 } else { next() * 2.0 + 1e-3 });
            let n = 1 + (next() * 300.0) as i64;
            let (k0, k1) = ((c - rad).floor() as i64, (c + rad).floor() as i64);
            let ranges = pixels_near(c, rad, k0..=k1, 1.0, start, span, n);
            assert!(ranges.windows(2).all(|w| w[0].1 + 1 < w[1].0), "{ranges:?}");
            assert!(ranges.iter().all(|r| 0 <= r.0 && r.0 <= r.1 && r.1 < n), "{ranges:?}");
            for i in 0..n {
                let u = (i as f32 + 0.5) / n as f32;
                let p = if span > 0.0 { start + u * span } else { u };
                let hit = (k0..=k1).any(|k| (p + k as f32 - c).abs() <= rad);
                assert!(
                    !hit || ranges.iter().any(|r| r.0 <= i && i <= r.1),
                    "pixel {i} of {n} at {p}: c {c} rad {rad} start {start} span {span} {ranges:?}"
                );
            }
        }
    }

    use super::*;

    fn run_framed(body: &str, size: usize, frame: Frame) -> Tex {
        let text = format!("size {size}\nsubgraph p\nmeta kind \"patterns\"\n{body}end\nnode 1 color r=0 g=0 b=0\noutput o 1\n");
        let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        let ctx = Context { lib: &g.subgraphs, size: [size, size], graph_inputs: &BTreeMap::new(), seed: 0, graph: None };
        render(g.subgraphs.get("p").unwrap(), &ctx, 0, &[], None, frame).unwrap()
    }

    fn run(body: &str, size: usize) -> Tex {
        run_framed(body, size, Frame::default())
    }

    fn at(t: &Tex, x: usize, y: usize) -> f32 {
        t.px[y * t.w + x][0]
    }

    const TWO: &str = "node 3 pp_quadrant draws=0 q0=@1 q3=@2\nnode 4 pp_out in=@3\n";

    #[test]
    fn an_image_pattern_reads_a_pyramid_level_or_as_the_filter_says() {
        // a white half on black, drawn smaller: read from a pyramid level the edge comes out gray, unfiltered not
        let draw = |filter: u32| {
            let g = format!(
                "size 64\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant pattern=0 draws=1 image=1 image_filter={filter} size_x=0.3 size_y=0.3\nnode 2 pp_out in=@1\nend\nnode 1 triangle_strips strips=\"0000000000007fffffff0000ffff7fff\"\nnode 2 pattern_program program=\"p\" in=@1\noutput o 2\n"
            );
            Graph::parse(&g).unwrap_or_else(|e| panic!("{e}")).evaluate(Some(64)).unwrap()["o"].clone()
        };
        let gray = |t: &Tex| t.px.iter().filter(|p| p[0] > 0.01 && p[0] < 0.99).count();
        assert!(gray(&draw(0)) > 0);
        assert_eq!(gray(&draw(2)), 0);
    }

    #[test]
    fn the_gaussian_is_the_measured_one() {
        let at = |r: f32| shape(4, r, 0.0, 0.0).unwrap();
        assert!((at(0.0) - 1.0).abs() < 2e-5);
        assert!((at(0.5) - (math::exp(-11.1f32 * 0.25) - math::exp(-11.1f32))).abs() < 1e-6);
        assert!(at(1.0).abs() < 1e-6, "reaches 0 at the rim");
        assert_eq!(shape(4, 0.8, 0.8, 0.0), Some(0.0), "outside the disc");
    }

    #[test]
    fn pattern_zero_is_a_square_filling_the_cell() {
        let t = run("node 1 pp_quadrant pattern=0 draws=1\nnode 2 pp_out in=@1\n", 16);
        assert_eq!((at(&t, 0, 0), at(&t, 15, 15)), (1.0, 1.0));
    }

    #[test]
    fn size_is_relative_to_the_cell() {
        let t = run("node 1 pp_quadrant pattern=0 draws=1 size_x=0.5 size_y=0.5\nnode 2 pp_quadrant draws=0 q0=@1\nnode 3 pp_out in=@2\n", 16);
        // the top-left cell is 8 wide, a half-size square in it covers pixels 2 to 5
        assert_eq!((at(&t, 2, 2) > 0.0, at(&t, 5, 5) > 0.0, at(&t, 1, 2), at(&t, 6, 5)), (true, true, 0.0, 0.0));
    }

    #[test]
    fn offsets_are_in_uv_and_only_the_branch_offset_reaches_children() {
        let child = "node 1 pp_quadrant pattern=0 draws=1 size_x=0.5 size_y=0.5\n";
        let branch = run(&format!("{child}node 2 pp_quadrant draws=0 branch_offset_x=0.25 branch_offset_y=0 q0=@1\nnode 3 pp_out in=@2\n"), 16);
        assert!(at(&branch, 6, 3) > 0.0 && at(&branch, 3, 3) == 0.0, "moved by 4 pixels");
        let own = run(&format!("{child}node 2 pp_quadrant draws=0 offset_x=0.25 offset_y=0 q0=@1\nnode 3 pp_out in=@2\n"), 16);
        assert!(at(&own, 3, 3) > 0.0 && at(&own, 6, 3) == 0.0, "stays where it was");
    }

    #[test]
    fn a_negative_size_flips_the_pattern() {
        let t = run("node 1 pp_quadrant pattern=8 draws=1 size_x=-1 size_y=1\nnode 2 pp_out in=@1\n", 16);
        assert!(at(&t, 1, 8) > at(&t, 14, 8));
    }

    #[test]
    fn the_level_follows_depth_and_roughness() {
        assert_eq!(depth_levels(1.0, 2), vec![1.0 / 3.0; 3]);
        assert_eq!(depth_levels(0.0, 3), vec![1.0; 4]);
        let l = depth_levels(0.5, 1);
        assert!((l[1] - 1.0 / 3.0).abs() < 1e-6, "{l:?}");
    }

    #[test]
    fn quadrants_that_draw_nothing_count_towards_the_depth_and_iterations_do_not() {
        let deep = run(
            "node 1 pp_quadrant pattern=0 draws=1\nnode 2 pp_quadrant draws=0\nnode 3 pp_quadrant draws=0 q0=@2 q3=@1\nnode 4 pp_quadrant draws=0 q0=@3\nnode 5 pp_out in=@4\n",
            16,
        );
        assert!((deep.px.iter().map(|p| p[0]).fold(0.0, f32::max) - 1.0 / 3.0).abs() < 1e-6);
        let iterated = run(
            "node 1 pp_quadrant pattern=0 draws=1\nnode 2 pp_iterate count=1 body=@1\nnode 3 pp_quadrant draws=0 q0=@2\nnode 4 pp_out in=@3\n",
            16,
        );
        assert_eq!(at(&iterated, 0, 0), 0.5);
    }

    #[test]
    fn instances_add_or_keep_the_larger_and_luminance_scales_and_subtracts() {
        let overlap = |a: f32, b: f32, flag: u8| {
            let body = format!(
                "node 1 pp_quadrant pattern=0 draws=1 luminance={a} max={flag} size_x=1.5 size_y=1.5\nnode 2 pp_quadrant pattern=0 draws=1 luminance={b} max={flag} size_x=1.5 size_y=1.5\n{TWO}"
            );
            at(&run(&body, 16), 8, 8)
        };
        assert!((overlap(0.6, 0.2, 0) - 0.4).abs() < 1e-6);
        assert!((overlap(0.6, 0.2, 1) - 0.3).abs() < 1e-6);
        assert!((overlap(1.0, -0.5, 0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn the_frame_gives_background_and_opacity() {
        let t = run_framed(
            "node 1 pp_quadrant pattern=0 draws=1 size_x=0.5 size_y=0.5\nnode 2 pp_out in=@1\n",
            16,
            Frame { background: 0.25, opacity: 0.5, ..Frame::default() },
        );
        assert_eq!(at(&t, 0, 0), 0.25);
        assert_eq!(at(&t, 8, 8), 0.75);
    }

    #[test]
    fn a_region_shows_part_of_the_pattern_space_and_inverted_axes_are_ignored() {
        let body = "node 1 pp_quadrant pattern=8 draws=1\nnode 2 pp_out in=@1\n";
        let t = run_framed(body, 16, Frame { region: [0.5, 0.50001, 0.0, 1.0], ..Frame::default() });
        assert!((at(&t, 0, 8) - 0.5).abs() < 0.01 && (at(&t, 15, 8) - 0.5).abs() < 0.01, "one column stretched across");
        let u = run_framed(body, 16, Frame { region: [1.0, 0.0, 0.0, 1.0], ..Frame::default() });
        assert!(at(&u, 15, 8) > at(&u, 0, 8), "an inverted range shows everything");
    }

    #[test]
    fn the_repeat_box_sets_how_far_apart_the_repeats_are() {
        let body = "node 1 pp_quadrant pattern=0 draws=1 size_x=0.5 size_y=0.5\nnode 2 pp_out in=@1\n";
        let lit = |t: &Tex| t.px.iter().filter(|p| p[0] > 0.5).count();
        // the canvas shows four units of pattern space per axis: one square in all of it, or one per unit
        let region = [-1.5, 2.5, -1.5, 2.5];
        assert_eq!(lit(&run_framed(body, 32, Frame { region, repeat: region, ..Frame::default() })), 4 * 4);
        assert_eq!(lit(&run_framed(body, 32, Frame { region, ..Frame::default() })), 4 * 4 * 16);
        // the same without a window: a repeat every half on x only, an empty box on y repeats every 1
        let t = run_framed(
            "node 1 pp_quadrant pattern=0 draws=1 size_x=0.25 size_y=0.25\nnode 2 pp_out in=@1\n",
            16,
            Frame { repeat: [0.0, 0.5, 0.0, 0.0], ..Frame::default() },
        );
        assert_eq!(lit(&t), 2 * 4 * 4);
    }

    #[test]
    fn a_pixel_centre_between_two_cells_belongs_to_the_top_left_one() {
        // four cells on a canvas of one pixel: its centre is the corner they share, and only the first draws
        let body = "node 1 pp_quadrant pattern=0 draws=1 luminance=0.25\nnode 2 pp_quadrant pattern=0 draws=1 luminance=0.5\nnode 3 pp_quadrant draws=0 q0=@1 q1=@2 q2=@2 q3=@2\nnode 4 pp_out in=@3\n";
        let t = run_framed(body, 1, Frame { roughness: 0.0, ..Frame::default() });
        assert!((at(&t, 0, 0) - 0.25).abs() < 1e-6, "{}", at(&t, 0, 0));
    }

    #[test]
    fn colour_instances_lay_over_by_their_alpha_or_add() {
        // half transparent red over a green image; alpha comes with the luminance expression
        let text = |max: u8| {
            format!(
                "size 4\nsubgraph f\nmeta kind \"function\"\nnode 2 f_vec4 x=1 y=0 z=0 w=0.5\nnode 1 f_out in=@2\nend\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant pattern=0 draws=1 max={max} fn.luminance=\"f\"\nnode 2 pp_out in=@1\nend\nnode 1 color r=0 g=0.5 b=0\nnode 2 pattern_program program=\"p\" color=1 bg=@1 roughness=0\noutput o 2\n"
            )
        };
        let px = |max: u8| Graph::parse(&text(max)).unwrap().evaluate(Some(4)).unwrap()["o"].px[0];
        let over = px(1);
        assert!((over[0] - 0.5).abs() < 1e-6 && (over[1] - 0.25).abs() < 1e-6, "{over:?}");
        let added = px(0);
        assert!((added[0] - 0.5).abs() < 1e-6 && (added[1] - 0.5).abs() < 1e-6, "{added:?}");
    }

    #[test]
    fn in_colour_a_shape_is_coverage_and_the_colour_is_the_luminances() {
        // a disc half covering a pixel lays its colour over by half, it doesn't darken it as well
        let text = "size 8\nsubgraph f\nmeta kind \"function\"\nnode 2 f_vec4 x=1 y=1 z=1 w=1\nnode 1 f_out in=@2\nend\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant pattern=3 draws=1 max=1 fn.luminance=\"f\"\nnode 2 pp_out in=@1\nend\nnode 1 color r=0 g=0 b=0\nnode 2 pattern_program program=\"p\" color=1 bg=@1 roughness=0\noutput o 2\n";
        let t = &Graph::parse(text).unwrap().evaluate(Some(8)).unwrap()["o"];
        let (i, p) = t.px.iter().copied().enumerate().find(|(_, p)| p[0] > 0.05 && p[0] < 0.95).expect("an edge pixel");
        // white over black by coverage a gives a, not a squared: the same disc drawn in grey is the coverage itself
        let grey = &Graph::parse(&text.replace(" fn.luminance=\"f\"", "").replace(" color=1", "")).unwrap().evaluate(Some(8)).unwrap()["o"];
        let coverage = grey.px[i][0];
        assert!((p[0] - coverage).abs() < 1e-4, "{p:?} against coverage {coverage}");
    }

    #[test]
    fn a_background_image_is_what_the_program_draws_on() {
        let text = "size 8\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant pattern=0 draws=1 size_x=0.5 size_y=0.5 luminance=0.25\nnode 2 pp_out in=@1\nend\nnode 1 color r=0.5 g=0.5 b=0.5\nnode 2 pattern_program program=\"p\" bg=@1 background=0.9\noutput o 2\n";
        let t = &Graph::parse(text).unwrap().evaluate(Some(8)).unwrap()["o"];
        assert!((at(t, 0, 0) - 0.5).abs() < 1e-6, "the image, not the background value");
        assert!((at(t, 4, 4) - 0.75).abs() < 1e-6, "and the pattern on it");
    }

    #[test]
    fn functions_see_the_size_and_depth_of_the_cell() {
        // luminance = cell size * depth: at depth 2 the cell is a quarter wide, so one half
        let text = "size 8\nsubgraph f\nmeta kind \"function\"\nnode 2 f_cell_size\nnode 3 f_cell_depth\nnode 4 f_mul a=@2 b=@3\nnode 1 f_out in=@4\nend\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant pattern=0 draws=1 fn.luminance=\"f\"\nnode 2 pp_quadrant draws=0 q0=@1\nnode 3 pp_quadrant draws=0 q0=@2\nnode 4 pp_out in=@3\nend\nnode 1 pattern_program program=\"p\" roughness=0\noutput o 1\n";
        let t = &Graph::parse(text).unwrap().evaluate(Some(8)).unwrap()["o"];
        assert!((at(t, 0, 0) - 0.5).abs() < 1e-6, "{}", at(t, 0, 0));
    }

    #[test]
    fn without_tiling_a_pattern_stops_at_the_edge() {
        let body = "node 1 pp_quadrant pattern=0 draws=1 size_x=0.5 size_y=0.5 offset_x=0.5 offset_y=0.5\nnode 2 pp_out in=@1\n";
        let t = run_framed(body, 16, Frame { tile: [false, false], ..Frame::default() });
        assert_eq!((at(&t, 0, 0), at(&t, 15, 15)), (0.0, 1.0));
    }

    #[test]
    fn patterns_wrap_around_the_canvas() {
        let t = run("node 1 pp_quadrant pattern=0 draws=1 size_x=0.5 size_y=0.5 offset_x=0.5 offset_y=0.5\nnode 2 pp_out in=@1\n", 16);
        assert_eq!((at(&t, 0, 0), at(&t, 15, 15), at(&t, 8, 8)), (1.0, 1.0, 0.0));
    }

    #[test]
    fn variation_reshapes_patterns_as_measured() {
        let near = |a: Option<f32>, b: f32| assert!((a.unwrap() - b).abs() < 0.02, "{a:?} vs {b}");
        near(shape(7, 0.5, 0.0, 1.0), 0.5);
        near(shape(7, 0.75, 0.0, 0.5), 0.5);
        near(shape(8, 1.0, 0.0, 0.25), 0.75);
        near(shape(15, -0.999, 0.0, 0.25), 0.25);
        near(shape(9, 0.5, 0.0, 0.25), 0.0);
        near(shape(10, 0.25, 0.0, 0.5), 0.38);
        near(shape(12, -0.625, 0.0, 0.0), 0.89);
        near(shape(13, 0.5, 0.0, 0.5), 0.74);
        near(shape(13, 0.0, 0.125, 1.0), 0.69);
        near(shape(16, 0.75, 0.0, 0.25), 0.91);
        assert_eq!(shape(0, 1.5, 0.0, 0.0), None);
        assert_eq!((shape(0, -1.0, 0.0, 0.0), shape(0, 1.0, 1.0, 0.0)), (None, Some(1.0)));
    }

    fn run_images(quadrant: &str, images: &[&Tex], size: usize) -> Tex {
        let text = format!(
            "size {size}\nsubgraph p\nmeta kind \"patterns\"\nnode 1 {quadrant}\nnode 2 pp_out in=@1\nend\nnode 1 color r=0 g=0 b=0\noutput o 1\n"
        );
        let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}"));
        let ctx = Context { lib: &g.subgraphs, size: [size, size], graph_inputs: &BTreeMap::new(), seed: 0, graph: None };
        render(g.subgraphs.get("p").unwrap(), &ctx, 0, images, None, Frame::default()).unwrap()
    }

    #[test]
    fn an_image_index_past_the_inputs_draws_the_last() {
        let (a, b) = (Tex::solid(2, 2, [0.25, 0.25, 0.25, 1.0]), Tex::solid(2, 2, [0.5, 0.5, 0.5, 1.0]));
        for (index, want) in [(0, 0.25), (1, 0.5), (2, 0.5), (5, 0.5)] {
            let t = run_images(&format!("pp_quadrant pattern=0 draws=1 image=1 image_filter=2 image_index={index}"), &[&a, &b], 8);
            assert_eq!(at(&t, 4, 4), want, "index {index}");
        }
    }

    #[test]
    fn an_image_pattern_fades_to_black_past_its_edges() {
        // a white 2 x 2 image over an 8 x 8 canvas, read bilinear: the corner pixel is 5/8 of the way from the black outside to
        // the first texel; read wrapped it would be white
        let white = Tex::solid(2, 2, [1.0; 4]);
        let t = run_images("pp_quadrant pattern=0 draws=1 image=1 image_filter=1", &[&white], 8);
        assert_eq!(at(&t, 0, 0), 0.625 * 0.625);
        assert_eq!(at(&t, 0, 4), 0.625);
        assert_eq!(at(&t, 4, 4), 1.0);
    }

    #[test]
    fn functions_share_variables_across_visits() {
        // each visit adds 0.25 to x and moves the square right by it
        let text = "size 16\nsubgraph step\nmeta kind \"function\"\nnode 1 f_get name=\"x\"\nnode 2 f_float v=0.25\nnode 3 f_add a=@1 b=@2\nnode 4 f_set a=@3 name=\"x\"\nnode 5 f_float v=0\nnode 6 f_vec2 x=@4 y=@5\nnode 7 f_out in=@6\nend\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant pattern=0 draws=1 size_x=0.125 size_y=0.125 fn.offset=\"step\"\nnode 2 pp_iterate count=2 body=@1\nnode 3 pp_out in=@2\nend\nnode 1 color r=0 g=0 b=0\noutput o 1\n";
        let g = Graph::parse(text).unwrap_or_else(|e| panic!("{e}"));
        let ctx = Context { lib: &g.subgraphs, size: [16, 16], graph_inputs: &BTreeMap::new(), seed: 0, graph: None };
        let t = render(g.subgraphs.get("p").unwrap(), &ctx, 0, &[], None, Frame::default()).unwrap();
        assert!(at(&t, 12, 8) > 0.0 && at(&t, 0, 8) > 0.0 && at(&t, 8, 8) == 0.0);
    }

    #[test]
    fn a_smaller_node_in_a_compiled_graph_reads_the_graphs_size() {
        // luminance f_size.x / 64 or f_own_size.x / 64 on a 16 x 16 node: the graph's 64 where there is one, else the node's 16
        let lum = |kind: &str, graph: Option<[usize; 2]>| {
            let text = format!(
                "size 16\nsubgraph l\nmeta kind \"function\"\nnode 2 {kind}\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 4 f_float v=64\nnode 5 f_div_large a=@3 b=@4\nnode 1 f_out in=@5\nend\nsubgraph p\nmeta kind \"patterns\"\nnode 1 pp_quadrant pattern=0 draws=1 fn.luminance=\"l\"\nnode 2 pp_out in=@1\nend\nnode 1 color r=0 g=0 b=0\noutput o 1\n"
            );
            let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}"));
            let ctx = Context { lib: &g.subgraphs, size: [16, 16], graph_inputs: &BTreeMap::new(), seed: 0, graph };
            let t = render(g.subgraphs.get("p").unwrap(), &ctx, 0, &[], None, Frame::default()).unwrap();
            at(&t, 8, 8)
        };
        assert_eq!(lum("f_size", Some([64, 64])), 1.0);
        assert_eq!(lum("f_own_size", Some([64, 64])), 0.25);
        assert_eq!(lum("f_size", None), 0.25);
    }
}

#[cfg(test)]
mod mip_tests {
    use super::mip_level;

    #[test]
    fn the_level_is_the_nearest_whole_one() {
        for (scale, level) in [(1.0, 0), (1.3, 0), (1.41, 0), (std::f32::consts::SQRT_2, 1), (1.7, 1), (2.3, 1), (2.9, 2), (5.5, 2), (0.5, 0)] {
            assert_eq!(mip_level(scale), level, "scale {scale}");
        }
    }
}
