// SPDX-License-Identifier: MIT OR Apache-2.0
//! Recursive pattern construction ("pattern programs").
//!
//! A pattern program is a small graph (meta `kind "fxmap"`) of `fxm_*` nodes that is walked from its `fxm_out` node.
//! The walk carries a *cell*: a centre, a size, a rotation, a value and a colour. Nodes either change the cell and
//! run children (`fxm_quadrant`, `fxm_iterate`, `fxm_switch`) or draw into the canvas (`fxm_marker`).
//! The `fx_map` node of a texture graph runs a program and rasterises the markers; the canvas wraps at its edges so
//! the result tiles.

use crate::graph::{Graph, Node};
use crate::math;
use crate::noise::rnd;
use crate::tex::{Tex, lum};

/// Most markers one pattern program may draw (a guard, not measured).
pub const MAX_MARKERS: usize = 500_000;

pub const FXM_KINDS: &[&str] = &["fxm_out", "fxm_marker", "fxm_quadrant", "fxm_iterate", "fxm_switch"];

pub fn is_program(g: &Graph) -> bool {
    g.meta.get("kind").map(|k| k == "fxmap").unwrap_or(false)
}

#[derive(Clone, Copy)]
struct Cell {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rot: f32,
    val: f32,
    col: [f32; 3],
}

/// Settings of one run.
pub struct Run<'a> {
    pub depth: u32,
    pub seed: u32,
    /// 0 max, 1 add, 2 over (alpha blend), 3 min, 4 multiply
    pub blend: u32,
    pub color: bool,
    pub patterns: &'a [Option<&'a Tex>; 4],
}

struct Walk<'a> {
    g: &'a Graph,
    run: &'a Run<'a>,
    canvas: Tex,
    markers: usize,
    counter: u32,
    over: bool,
}

fn n(node: &Node, k: &str, d: f32) -> f32 {
    node.nums.get(k).copied().unwrap_or(d)
}

impl<'a> Walk<'a> {
    fn rand(&mut self, salt: u32) -> f32 {
        self.counter = self.counter.wrapping_add(1);
        rnd(self.counter as i32, salt as i32, self.run.seed)
    }

    fn signed(&mut self, salt: u32) -> f32 {
        self.rand(salt) * 2.0 - 1.0
    }

    fn visit(&mut self, id: u32, cell: Cell, depth: u32) {
        if depth > self.run.depth || self.markers >= MAX_MARKERS || self.over {
            return;
        }
        let Some(node) = self.g.nodes.get(&id) else { return };
        match node.kind.as_str() {
            "fxm_out" => {
                if let Some(c) = node.ins.get("in") {
                    self.visit(*c, cell, depth + 1);
                }
            }
            "fxm_marker" => self.marker(node, cell),
            "fxm_quadrant" => {
                let (sin, cos) = math::sin_cos(cell.rot);
                let scale = n(node, "scale", 0.5).max(0.0);
                for q in 0..4usize {
                    let Some(child) = node.ins.get(&format!("q{q}")) else { continue };
                    if self.rand(10 + q as u32) > n(node, &format!("p{q}"), 1.0) {
                        continue;
                    }
                    // quadrant centres at (-1/4, -1/4), (1/4, -1/4), (-1/4, 1/4), (1/4, 1/4) of the cell
                    let (qx, qy) = (if q % 2 == 0 { -0.25 } else { 0.25 }, if q < 2 { -0.25 } else { 0.25 });
                    let (ox, oy) = (qx * cell.w + n(node, "off_x", 0.0) * cell.w, qy * cell.h + n(node, "off_y", 0.0) * cell.h);
                    let rx = n(node, "rnd_pos", 0.0) * cell.w * self.signed(20 + q as u32);
                    let ry = n(node, "rnd_pos", 0.0) * cell.h * self.signed(30 + q as u32);
                    let rs = 1.0 + n(node, "rnd_scale", 0.0) * self.signed(40 + q as u32);
                    let rr = n(node, "rnd_rot", 0.0).to_radians() * self.signed(50 + q as u32);
                    let rv = 1.0 + n(node, "rnd_val", 0.0) * self.signed(60 + q as u32);
                    let c = Cell {
                        x: cell.x + (ox + rx) * cos - (oy + ry) * sin,
                        y: cell.y + (ox + rx) * sin + (oy + ry) * cos,
                        w: cell.w * scale * rs,
                        h: cell.h * scale * rs,
                        rot: cell.rot + n(node, &format!("r{q}"), 0.0).to_radians() + rr,
                        val: cell.val * n(node, &format!("v{q}"), 1.0) * rv,
                        col: cell.col,
                    };
                    self.visit(*child, c, depth + 1);
                }
            }
            "fxm_iterate" => {
                let Some(body) = node.ins.get("body").copied() else { return };
                let (gx, gy) = (n(node, "grid_x", 0.0).round() as i32, n(node, "grid_y", 0.0).round() as i32);
                let count = if gx > 0 && gy > 0 { gx * gy } else { n(node, "count", 4.0).round().clamp(0.0, 100_000.0) as i32 };
                let (sin, cos) = math::sin_cos(cell.rot);
                let mut cur = cell;
                for i in 0..count {
                    if self.markers >= MAX_MARKERS {
                        break;
                    }
                    let mut c = cur;
                    if gx > 0 && gy > 0 {
                        // a grid of equal cells filling the current one
                        let (ix, iy) = (i % gx, i / gx);
                        let stagger = if iy % 2 == 1 { n(node, "stagger", 0.0) } else { 0.0 };
                        let (lx, ly) = (((ix as f32 + 0.5 + stagger) / gx as f32 - 0.5) * cell.w, ((iy as f32 + 0.5) / gy as f32 - 0.5) * cell.h);
                        c.x = cell.x + lx * cos - ly * sin;
                        c.y = cell.y + lx * sin + ly * cos;
                        c.w = cell.w / gx as f32;
                        c.h = cell.h / gy as f32;
                    }
                    let rx = n(node, "rnd_x", 0.0) * cur.w * self.signed(70);
                    let ry = n(node, "rnd_y", 0.0) * cur.h * self.signed(71);
                    c.x += rx * cos - ry * sin;
                    c.y += rx * sin + ry * cos;
                    let rs = 1.0 + n(node, "rnd_scale", 0.0) * self.signed(72);
                    c.w *= rs;
                    c.h *= rs;
                    c.rot += n(node, "rnd_rot", 0.0).to_radians() * self.signed(73);
                    c.val *= 1.0 + n(node, "rnd_val", 0.0) * self.signed(74);
                    self.visit(body, c, depth + 1);
                    // per-iteration change of the running cell
                    let (dx, dy) = (n(node, "dx", 0.0) * cur.w, n(node, "dy", 0.0) * cur.h);
                    let (s2, c2) = math::sin_cos(cur.rot);
                    cur.x += dx * c2 - dy * s2;
                    cur.y += dx * s2 + dy * c2;
                    let ds = n(node, "dscale", 1.0);
                    cur.w *= ds;
                    cur.h *= ds;
                    cur.rot += n(node, "drot", 0.0).to_radians();
                    cur.val *= n(node, "dval", 1.0);
                }
            }
            "fxm_switch" => {
                let slots = ["a", "b", "c", "d"];
                let live: Vec<usize> = slots.iter().enumerate().filter(|(_, s)| node.ins.contains_key(**s)).map(|(i, _)| i).collect();
                let Some(last) = live.last().copied() else { return };
                let pick = if n(node, "mode", 0.0) > 0.5 {
                    // weighted random choice among the connected slots
                    let total: f32 = live.iter().map(|i| n(node, &format!("w{i}"), 1.0).max(0.0)).sum();
                    if total <= 0.0 {
                        return;
                    }
                    let mut r = self.rand(80) * total;
                    let mut chosen = last;
                    for i in &live {
                        r -= n(node, &format!("w{i}"), 1.0).max(0.0);
                        if r <= 0.0 {
                            chosen = *i;
                            break;
                        }
                    }
                    chosen
                } else {
                    let want = n(node, "index", 0.0).round().max(0.0) as usize;
                    live.get(want % live.len()).copied().unwrap_or(last)
                };
                if let Some(child) = slots.get(pick).and_then(|s| node.ins.get(*s)) {
                    self.visit(*child, cell, depth + 1);
                }
            }
            _ => {}
        }
    }

    fn marker(&mut self, node: &Node, cell: Cell) {
        self.markers += 1;
        let (px, py) = (n(node, "x", 0.0), n(node, "y", 0.0));
        let (sin, cos) = math::sin_cos(cell.rot);
        let rx = n(node, "rnd_pos", 0.0) * self.signed(90);
        let ry = n(node, "rnd_pos", 0.0) * self.signed(91);
        let ox = (px + rx) * cell.w;
        let oy = (py + ry) * cell.h;
        let cx = cell.x + ox * cos - oy * sin;
        let cy = cell.y + ox * sin + oy * cos;
        let rs = 1.0 + n(node, "rnd_scale", 0.0) * self.signed(92);
        let (w, h) = (cell.w * n(node, "w", 1.0) * rs, cell.h * n(node, "h", 1.0) * rs);
        let rot = cell.rot + n(node, "rot", 0.0).to_radians() + n(node, "rnd_rot", 0.0).to_radians() * self.signed(93);
        let val = (cell.val * n(node, "value", 1.0) * (1.0 + n(node, "rnd_val", 0.0) * self.signed(94))).max(0.0);
        let mut col = cell.col;
        let rc = n(node, "rnd_color", 0.0);
        let tint = [n(node, "r", 1.0), n(node, "g", 1.0), n(node, "b", 1.0)];
        for (k, (c, tint)) in col.iter_mut().zip(tint).enumerate() {
            *c *= tint * (1.0 + rc * self.signed(95 + k as u32));
        }
        let shape = n(node, "shape", 0.0).round() as i32;
        let soft = n(node, "softness", 0.02).clamp(0.0, 1.0);
        let src = n(node, "src", 0.0).round().clamp(0.0, 3.0) as usize;
        let pattern = if shape == 3 { self.run.patterns.get(src).copied().flatten() } else { None };
        let (cw, ch) = (self.canvas.w as i32, self.canvas.h as i32);
        let (pxw, pxh) = (cw as f32, ch as f32);
        // pixel bounding circle of the rotated rectangle
        let rad = 0.5 * (w * w + h * h).sqrt();
        if rad <= 0.0 || !rad.is_finite() {
            return;
        }
        let (x0, x1) = (((cx - rad) * pxw).floor() as i32, ((cx + rad) * pxw).ceil() as i32);
        let (y0, y1) = (((cy - rad) * pxh).floor() as i32, ((cy + rad) * pxh).ceil() as i32);
        if (x1 - x0) > cw * 2 || (y1 - y0) > ch * 2 {
            return;
        }
        let (rs_, rc_) = math::sin_cos(-rot);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (u, v) = ((x as f32 + 0.5) / pxw - cx, (y as f32 + 0.5) / pxh - cy);
                let (lx, ly) = ((u * rc_ - v * rs_) / (w * 0.5).max(1e-6), (u * rs_ + v * rc_) / (h * 0.5).max(1e-6));
                let (cover, pat_col) = match shape {
                    1 => (edge(lx.abs().max(ly.abs()), soft), None),
                    2 => (edge(lx.abs() + ly.abs(), soft), None),
                    3 => {
                        if lx.abs() > 1.0 || ly.abs() > 1.0 {
                            (0.0, None)
                        } else if let Some(t) = pattern {
                            let c = t.sample(lx * 0.5 + 0.5, ly * 0.5 + 0.5);
                            (lum(c) * c[3], Some([c[0], c[1], c[2]]))
                        } else {
                            (1.0, None)
                        }
                    }
                    _ => (edge((lx * lx + ly * ly).sqrt(), soft), None),
                };
                if cover <= 0.0 {
                    continue;
                }
                let (wx, wy) = (x.rem_euclid(cw) as usize, y.rem_euclid(ch) as usize);
                let Some(dst) = self.canvas.px.get_mut(wy * cw as usize + wx) else { continue };
                let src_c = match (self.run.color, pat_col) {
                    (true, Some(p)) => [p[0] * col[0], p[1] * col[1], p[2] * col[2]],
                    (true, None) => [col[0], col[1], col[2]],
                    (false, Some(p)) => {
                        let l = (p[0] + p[1] + p[2]) / 3.0;
                        [l, l, l]
                    }
                    (false, None) => [1.0, 1.0, 1.0],
                };
                for (d, src_c) in dst.iter_mut().zip(src_c) {
                    let sv = src_c * val;
                    let s = sv * if pat_col.is_some() { 1.0 } else { cover };
                    let c = cover.min(1.0);
                    *d = match self.run.blend {
                        1 => *d + s,
                        2 => *d + (sv - *d) * c,
                        3 => *d + (d.min(sv) - *d) * c,
                        4 => *d * (1.0 + (sv - 1.0) * c),
                        _ => d.max(s),
                    };
                }
                dst[3] = 1.0;
            }
        }
    }
}

/// 1 inside, 0 outside, soft over the outer `soft` of the unit radius.
fn edge(d: f32, soft: f32) -> f32 {
    if d >= 1.0 {
        0.0
    } else if soft <= 1e-4 || d <= 1.0 - soft {
        1.0
    } else {
        let t = (1.0 - d) / soft;
        t * t * (3.0 - 2.0 * t)
    }
}

/// Run `program` on a `w` by `h` canvas filled with `background`.
pub fn render(program: &Graph, w: usize, h: usize, background: [f32; 3], run: &Run) -> Result<Tex, String> {
    let outs: Vec<u32> = program.nodes.iter().filter(|(_, n)| n.kind == "fxm_out").map(|(i, _)| *i).collect();
    let root = match outs.as_slice() {
        [one] => *one,
        [] => return Err("the pattern program has no Output node".into()),
        _ => return Err("the pattern program has more than one Output node".into()),
    };
    for (id, node) in &program.nodes {
        if !FXM_KINDS.contains(&node.kind.as_str()) {
            return Err(format!("node {id} ({}) is not a pattern-program node", node.kind));
        }
    }
    program.check_cycles().map_err(|e| e.to_string())?;
    let mut walk =
        Walk { g: program, run, canvas: Tex::solid(w, h, [background[0], background[1], background[2], 1.0]), markers: 0, counter: 0, over: false };
    let start = Cell { x: 0.5, y: 0.5, w: 1.0, h: 1.0, rot: 0.0, val: 1.0, col: [1.0, 1.0, 1.0] };
    walk.visit(root, start, 0);
    if walk.markers >= MAX_MARKERS {
        return Err(format!("the pattern program draws more than {MAX_MARKERS} markers"));
    }
    Ok(walk.canvas)
}

pub const FXM_SPECS: &[crate::graph::Spec] = &[
    crate::graph::Spec { kind: "fxm_out", req: &["in"], opt: &[], doc: "start of a pattern program" },
    crate::graph::Spec {
        kind: "fxm_marker",
        req: &[],
        opt: &[],
        doc: "draw a shape: shape (0 circle 1 square 2 diamond 3 image) src x y w h rot value r g b softness rnd_pos rnd_scale rnd_rot rnd_val rnd_color",
    },
    crate::graph::Spec {
        kind: "fxm_quadrant",
        req: &[],
        opt: &["q0", "q1", "q2", "q3"],
        doc: "split the cell in four: scale off_x off_y p0..p3 (chance) v0..v3 (value) r0..r3 (rotation) rnd_pos rnd_scale rnd_rot rnd_val",
    },
    crate::graph::Spec {
        kind: "fxm_iterate",
        req: &[],
        opt: &["body"],
        doc: "repeat body: count dx dy dscale drot dval, or grid_x grid_y stagger; rnd_x rnd_y rnd_scale rnd_rot rnd_val",
    },
    crate::graph::Spec { kind: "fxm_switch", req: &[], opt: &["a", "b", "c", "d"], doc: "run one branch: mode (0 index, 1 random) index w0..w3" },
];
