// SPDX-License-Identifier: MIT OR Apache-2.0
//! Vector graphics: path flattening, an anti-aliased scanline filler, simple strokes and a reader for a practical
//! subset of SVG (shapes, paths, transforms, solid fill and stroke colours).
//!
//! Not supported: gradients, patterns, clipping, masks, filters, text elements, `use` references and CSS style sheets.
//! Unsupported parts are skipped, never an error. Stroke joins are always round, caps are butt.

use crate::math;
use crate::tex::Tex;

pub type P = [f32; 2];

/// Affine transform [a b c d e f]: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, Debug)]
pub struct Xf(pub [f32; 6]);

impl Xf {
    pub const ID: Xf = Xf([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    pub fn then(self, o: Xf) -> Xf {
        // self applied first, then o
        let (a, b) = (self.0, o.0);
        Xf([
            b[0] * a[0] + b[2] * a[1],
            b[1] * a[0] + b[3] * a[1],
            b[0] * a[2] + b[2] * a[3],
            b[1] * a[2] + b[3] * a[3],
            b[0] * a[4] + b[2] * a[5] + b[4],
            b[1] * a[4] + b[3] * a[5] + b[5],
        ])
    }
    pub fn apply(&self, p: P) -> P {
        let m = self.0;
        [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
    }
    pub fn translate(x: f32, y: f32) -> Xf {
        Xf([1.0, 0.0, 0.0, 1.0, x, y])
    }
    pub fn scale(x: f32, y: f32) -> Xf {
        Xf([x, 0.0, 0.0, y, 0.0, 0.0])
    }
    pub fn rotate(rad: f32) -> Xf {
        let (s, c) = math::sin_cos(rad);
        Xf([c, s, -s, c, 0.0, 0.0])
    }
    /// average scale factor, used to scale stroke widths
    pub fn mean_scale(&self) -> f32 {
        let m = self.0;
        (m[0] * m[3] - m[1] * m[2]).abs().sqrt()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(P),
    Line(P),
    Quad(P, P),
    Cubic(P, P, P),
    Close,
}

/// A closed or open polyline.
#[derive(Clone, Debug)]
pub struct Poly {
    pub pts: Vec<P>,
    pub closed: bool,
}

/// Flatten segments into polylines; curves are split until they are within about 0.1 pixel.
pub fn flatten(segs: &[Seg], xf: &Xf) -> Vec<Poly> {
    let mut out: Vec<Poly> = Vec::new();
    let mut cur: Vec<P> = Vec::new();
    let mut start = [0.0f32; 2];
    let mut last = [0.0f32; 2];
    let flush = |cur: &mut Vec<P>, out: &mut Vec<Poly>, closed: bool| {
        if cur.len() >= 2 || (closed && !cur.is_empty()) {
            out.push(Poly { pts: std::mem::take(cur), closed });
        } else {
            cur.clear();
        }
    };
    for s in segs {
        match *s {
            Seg::Move(p) => {
                flush(&mut cur, &mut out, false);
                let q = xf.apply(p);
                cur.push(q);
                start = q;
                last = q;
            }
            Seg::Line(p) => {
                let q = xf.apply(p);
                if cur.is_empty() {
                    cur.push(last);
                }
                cur.push(q);
                last = q;
            }
            Seg::Quad(c, p) => {
                let (c, p) = (xf.apply(c), xf.apply(p));
                if cur.is_empty() {
                    cur.push(last);
                }
                let c1 = [last[0] + 2.0 / 3.0 * (c[0] - last[0]), last[1] + 2.0 / 3.0 * (c[1] - last[1])];
                let c2 = [p[0] + 2.0 / 3.0 * (c[0] - p[0]), p[1] + 2.0 / 3.0 * (c[1] - p[1])];
                cubic(last, c1, c2, p, &mut cur);
                last = p;
            }
            Seg::Cubic(a, b, p) => {
                let (a, b, p) = (xf.apply(a), xf.apply(b), xf.apply(p));
                if cur.is_empty() {
                    cur.push(last);
                }
                cubic(last, a, b, p, &mut cur);
                last = p;
            }
            Seg::Close => {
                flush(&mut cur, &mut out, true);
                last = start;
            }
        }
    }
    flush(&mut cur, &mut out, false);
    out
}

fn cubic(p0: P, p1: P, p2: P, p3: P, out: &mut Vec<P>) {
    // number of pieces from the control polygon length
    let d = |a: P, b: P| (math::powi(a[0] - b[0], 2) + math::powi(a[1] - b[1], 2)).sqrt();
    let len = d(p0, p1) + d(p1, p2) + d(p2, p3);
    let n = ((len * 0.5).sqrt() * 1.5).ceil().clamp(2.0, 200.0) as usize;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let (a, b, c, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push([a * p0[0] + b * p1[0] + c * p2[0] + e * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + e * p3[1]]);
    }
}

/// Anti-aliased coverage mask.
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub a: Vec<f32>,
}

const SUB: usize = 4;

/// Fill polygons. `even_odd` selects the fill rule (otherwise non-zero winding).
pub fn fill(polys: &[Poly], w: usize, h: usize, even_odd: bool) -> Mask {
    let mut m = Mask { w, h, a: vec![0.0; w * h] };
    // edges (x0, y0, x1, y1, dir) with y0 < y1
    let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
    for p in polys {
        let n = p.pts.len();
        if n < 2 {
            continue;
        }
        for (&a, &b) in p.pts.iter().zip(p.pts.iter().cycle().skip(1)) {
            if !(a[0].is_finite() && a[1].is_finite() && b[0].is_finite() && b[1].is_finite()) {
                continue;
            }
            if (a[1] - b[1]).abs() < 1e-9 {
                continue;
            }
            if a[1] < b[1] {
                edges.push((a[0], a[1], b[0], b[1], 1));
            } else {
                edges.push((b[0], b[1], a[0], a[1], -1));
            }
        }
    }
    if edges.is_empty() {
        return m;
    }
    edges.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut cross: Vec<(f32, i32)> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let weight = 1.0 / SUB as f32;
    for row in 0..h {
        for s in 0..SUB {
            let y = row as f32 + (s as f32 + 0.5) / SUB as f32;
            while edges.get(next).is_some_and(|e| e.1 <= y) {
                active.push(next);
                next += 1;
            }
            active.retain(|&e| edges.get(e).is_some_and(|e| e.3 > y));
            cross.clear();
            for &e in &active {
                let Some(&(x0, y0, x1, y1, d)) = edges.get(e) else { continue };
                cross.push((x0 + (x1 - x0) * (y - y0) / (y1 - y0), d));
            }
            cross.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut wind = 0i32;
            for (i, &(xa, ci)) in cross.iter().enumerate() {
                let before = wind;
                wind += ci;
                let inside_before = if even_odd { before & 1 != 0 } else { before != 0 };
                let inside_after = if even_odd { wind & 1 != 0 } else { wind != 0 };
                if !inside_before && inside_after {
                    // span starts here; find where it ends
                    let mut w2 = wind;
                    let mut j = i + 1;
                    while let Some(&(_, cj)) = cross.get(j) {
                        w2 += cj;
                        let inside = if even_odd { w2 & 1 != 0 } else { w2 != 0 };
                        if !inside {
                            break;
                        }
                        j += 1;
                    }
                    let xb = cross.get(j).map_or(w as f32, |c| c.0);
                    if let Some(r) = m.a.get_mut(row * w..(row + 1) * w) {
                        add_span(r, xa, xb, weight);
                    }
                }
            }
        }
    }
    for v in m.a.iter_mut() {
        *v = v.min(1.0);
    }
    m
}

fn add_span(row: &mut [f32], xa: f32, xb: f32, weight: f32) {
    let w = row.len() as f32;
    let (xa, xb) = (xa.clamp(0.0, w), xb.clamp(0.0, w));
    if xb <= xa {
        return;
    }
    let (ia, ib) = (xa.floor() as usize, (xb.ceil() as usize).min(row.len()));
    for (i, px) in row.iter_mut().enumerate().take(ib).skip(ia) {
        let l = (xa.max(i as f32)).min(xb);
        let r = (xb.min(i as f32 + 1.0)).max(xa);
        *px += (r - l).max(0.0) * weight;
    }
}

/// Outline of a stroked polyline as a set of consistently wound polygons (quads for the segments, discs at the joints).
pub fn stroke(polys: &[Poly], width: f32) -> Vec<Poly> {
    let hw = (width * 0.5).max(0.01);
    let mut out = Vec::new();
    let disc = |c: P, out: &mut Vec<Poly>| {
        let n = ((hw * 3.0).ceil() as usize).clamp(8, 48);
        let pts = (0..n).map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [c[0] + hw * math::cos(a), c[1] + hw * math::sin(a)]
        });
        out.push(Poly { pts: pts.collect(), closed: true });
    };
    for p in polys {
        let n = p.pts.len();
        let segs = if p.closed { n } else { n.saturating_sub(1) };
        for (&a, &b) in p.pts.iter().zip(p.pts.iter().cycle().skip(1)).take(segs) {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = (dx * dx + dy * dy).sqrt();
            if l < 1e-6 {
                continue;
            }
            let (nx, ny) = (-dy / l * hw, dx / l * hw);
            // quad wound the same way as the discs (counter-clockwise in y-down screen terms)
            out.push(Poly {
                pts: vec![[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]],
                closed: true,
            });
        }
        // joints
        let first = if p.closed { 0 } else { 1 };
        let last = if p.closed { n } else { n.saturating_sub(1) };
        for &q in p.pts.get(first..last).unwrap_or_default() {
            disc(q, &mut out);
        }
        if let [q] = *p.pts.as_slice() {
            disc(q, &mut out);
        }
    }
    // normalise winding so overlapping pieces add up instead of cancelling
    for q in out.iter_mut() {
        let mut area = 0.0;
        for (a, b) in q.pts.iter().zip(q.pts.iter().cycle().skip(1)) {
            area += a[0] * b[1] - b[0] * a[1];
        }
        if area < 0.0 {
            q.pts.reverse();
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------ path data

/// Parse SVG path data into segments (absolute coordinates). Arcs become cubic curves.
pub fn parse_path(d: &str) -> Vec<Seg> {
    let b: Vec<char> = d.chars().collect();
    let mut i = 0usize;
    let mut segs = Vec::new();
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut last_ctrl: Option<P> = None; // previous cubic control (for S)
    let mut last_q: Option<P> = None; // previous quad control (for T)
    let mut cmd = ' ';
    fn skip(b: &[char], i: &mut usize) {
        while b.get(*i).is_some_and(|&c| c.is_whitespace() || c == ',') {
            *i += 1;
        }
    }
    fn number(b: &[char], i: &mut usize) -> Option<f32> {
        skip(b, i);
        let s = *i;
        if b.get(*i).is_some_and(|&c| c == '+' || c == '-') {
            *i += 1;
        }
        let mut dot = false;
        while let Some(&c) = b.get(*i)
            && (c.is_ascii_digit() || (c == '.' && !dot))
        {
            if c == '.' {
                dot = true;
            }
            *i += 1;
        }
        if b.get(*i).is_some_and(|&c| c == 'e' || c == 'E') {
            let save = *i;
            *i += 1;
            if b.get(*i).is_some_and(|&c| c == '+' || c == '-') {
                *i += 1;
            }
            let ds = *i;
            while b.get(*i).is_some_and(|c| c.is_ascii_digit()) {
                *i += 1;
            }
            if ds == *i {
                *i = save;
            }
        }
        if s == *i {
            return None;
        }
        b.get(s..*i)?.iter().collect::<String>().parse::<f32>().ok().filter(|v| v.is_finite())
    }
    fn flag(b: &[char], i: &mut usize) -> Option<bool> {
        skip(b, i);
        if let Some(&c) = b.get(*i)
            && (c == '0' || c == '1')
        {
            *i += 1;
            Some(c == '1')
        } else {
            None
        }
    }
    loop {
        skip(&b, &mut i);
        let Some(&c) = b.get(i) else { break };
        if c.is_ascii_alphabetic() {
            cmd = c;
            i += 1;
            if cmd == 'z' || cmd == 'Z' {
                segs.push(Seg::Close);
                cx = sx;
                cy = sy;
                last_ctrl = None;
                last_q = None;
                continue;
            }
        } else if cmd == ' ' {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        macro_rules! num {
            () => {
                match number(&b, &mut i) {
                    Some(v) => v,
                    None => return segs,
                }
            };
        }
        match cmd.to_ascii_uppercase() {
            'M' => {
                let (x, y) = (num!() + ox, num!() + oy);
                segs.push(Seg::Move([x, y]));
                cx = x;
                cy = y;
                sx = x;
                sy = y;
                cmd = if rel { 'l' } else { 'L' };
                last_ctrl = None;
                last_q = None;
            }
            'L' => {
                let (x, y) = (num!() + ox, num!() + oy);
                segs.push(Seg::Line([x, y]));
                cx = x;
                cy = y;
                last_ctrl = None;
                last_q = None;
            }
            'H' => {
                let x = num!() + ox;
                segs.push(Seg::Line([x, cy]));
                cx = x;
                last_ctrl = None;
                last_q = None;
            }
            'V' => {
                let y = num!() + oy;
                segs.push(Seg::Line([cx, y]));
                cy = y;
                last_ctrl = None;
                last_q = None;
            }
            'C' => {
                let (x1, y1, x2, y2, x, y) = (num!() + ox, num!() + oy, num!() + ox, num!() + oy, num!() + ox, num!() + oy);
                segs.push(Seg::Cubic([x1, y1], [x2, y2], [x, y]));
                last_ctrl = Some([x2, y2]);
                last_q = None;
                cx = x;
                cy = y;
            }
            'S' => {
                let (x2, y2, x, y) = (num!() + ox, num!() + oy, num!() + ox, num!() + oy);
                let c1 = last_ctrl.map(|c| [2.0 * cx - c[0], 2.0 * cy - c[1]]).unwrap_or([cx, cy]);
                segs.push(Seg::Cubic(c1, [x2, y2], [x, y]));
                last_ctrl = Some([x2, y2]);
                last_q = None;
                cx = x;
                cy = y;
            }
            'Q' => {
                let (x1, y1, x, y) = (num!() + ox, num!() + oy, num!() + ox, num!() + oy);
                segs.push(Seg::Quad([x1, y1], [x, y]));
                last_q = Some([x1, y1]);
                last_ctrl = None;
                cx = x;
                cy = y;
            }
            'T' => {
                let (x, y) = (num!() + ox, num!() + oy);
                let c = last_q.map(|c| [2.0 * cx - c[0], 2.0 * cy - c[1]]).unwrap_or([cx, cy]);
                segs.push(Seg::Quad(c, [x, y]));
                last_q = Some(c);
                last_ctrl = None;
                cx = x;
                cy = y;
            }
            'A' => {
                let (rx, ry, rot) = (num!(), num!(), num!());
                let (large, sweep) = match (flag(&b, &mut i), flag(&b, &mut i)) {
                    (Some(a), Some(c)) => (a, c),
                    _ => return segs,
                };
                let (x, y) = (num!() + ox, num!() + oy);
                arc(&mut segs, [cx, cy], rx, ry, rot, large, sweep, [x, y]);
                cx = x;
                cy = y;
                last_ctrl = None;
                last_q = None;
            }
            _ => return segs,
        }
    }
    segs
}

#[allow(clippy::too_many_arguments)]
fn arc(segs: &mut Vec<Seg>, p0: P, rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool, p1: P) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if (p0[0] == p1[0] && p0[1] == p1[1]) || rx < 1e-6 || ry < 1e-6 {
        segs.push(Seg::Line(p1));
        return;
    }
    let phi = rot_deg.to_radians();
    let (sp, cp) = math::sin_cos(phi);
    let (dx, dy) = ((p0[0] - p1[0]) * 0.5, (p0[1] - p1[1]) * 0.5);
    let (x1, y1) = (cp * dx + sp * dy, -sp * dx + cp * dy);
    let lam = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lam > 1.0 {
        let s = lam.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = if den > 0.0 { (num / den).max(0.0).sqrt() } else { 0.0 };
    if large == sweep {
        k = -k;
    }
    let (cxp, cyp) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let (cx, cy) = (cp * cxp - sp * cyp + (p0[0] + p1[0]) * 0.5, sp * cxp + cp * cyp + (p0[1] + p1[1]) * 0.5);
    let ang = |ux: f32, uy: f32, vx: f32, vy: f32| math::atan2(ux * vy - uy * vx, ux * vx + uy * vy);
    let th1 = ang(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut dth = ang((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && dth > 0.0 {
        dth -= std::f32::consts::TAU;
    } else if sweep && dth < 0.0 {
        dth += std::f32::consts::TAU;
    }
    let n = (dth.abs() / (std::f32::consts::FRAC_PI_2)).ceil().max(1.0) as usize;
    let step = dth / n as f32;
    let t = 4.0 / 3.0 * math::tan(step / 4.0);
    let pt = |a: f32| -> (P, P) {
        let (s, c) = math::sin_cos(a);
        // point and derivative on the unrotated ellipse, then rotated and moved
        let x = rx * c;
        let y = ry * s;
        let dx = -rx * s;
        let dy = ry * c;
        ([cp * x - sp * y + cx, sp * x + cp * y + cy], [cp * dx - sp * dy, sp * dx + cp * dy])
    };
    for i in 0..n {
        let (a0, a1) = (th1 + step * i as f32, th1 + step * (i + 1) as f32);
        let ((pa, da), (pb, db)) = (pt(a0), pt(a1));
        let end = if i == n - 1 { p1 } else { pb };
        segs.push(Seg::Cubic([pa[0] + t * da[0], pa[1] + t * da[1]], [pb[0] - t * db[0], pb[1] - t * db[1]], end));
    }
}

// ------------------------------------------------------------------------------------------------ SVG subset

#[derive(Clone, Debug)]
struct Style {
    fill: Option<[f32; 3]>,
    stroke: Option<[f32; 3]>,
    stroke_w: f32,
    opacity: f32,
    fill_op: f32,
    stroke_op: f32,
    even_odd: bool,
    color: [f32; 3],
}

impl Default for Style {
    fn default() -> Self {
        Style { fill: Some([0.0; 3]), stroke: None, stroke_w: 1.0, opacity: 1.0, fill_op: 1.0, stroke_op: 1.0, even_odd: false, color: [0.0; 3] }
    }
}

struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    open: bool,
    close: bool,
}

fn tags(src: &str) -> Vec<Tag> {
    let b: Vec<char> = src.chars().collect();
    let at = |i: usize| b.get(i).copied();
    let text = |s: usize, e: usize| -> String { b.get(s..e).unwrap_or_default().iter().collect() };
    let mut i = 0;
    let mut out = Vec::new();
    while let Some(c0) = at(i) {
        if c0 != '<' {
            i += 1;
            continue;
        }
        // comments, declarations, CDATA
        if b.get(i..).is_some_and(|r| r.starts_with(&['<', '!', '-', '-'])) {
            let mut j = i + 4;
            while j + 2 < b.len() && b.get(j..j + 3) != Some(&['-', '-', '>'][..]) {
                j += 1;
            }
            i = j + 3;
            continue;
        }
        if at(i + 1).is_some_and(|c| c == '!' || c == '?') {
            while at(i).is_some_and(|c| c != '>') {
                i += 1;
            }
            i += 1;
            continue;
        }
        i += 1;
        let closing = at(i) == Some('/');
        if closing {
            i += 1;
        }
        let s = i;
        while at(i).is_some_and(|c| !c.is_whitespace() && c != '>' && c != '/') {
            i += 1;
        }
        let name: String = text(s, i);
        let mut attrs = Vec::new();
        let mut selfclose = false;
        while let Some(c) = at(i)
            && c != '>'
        {
            if c == '/' {
                selfclose = true;
                i += 1;
                continue;
            }
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            let ks = i;
            while at(i).is_some_and(|c| c != '=' && !c.is_whitespace() && c != '>' && c != '/') {
                i += 1;
            }
            let key: String = text(ks, i);
            while at(i).is_some_and(char::is_whitespace) {
                i += 1;
            }
            if at(i) == Some('=') {
                i += 1;
                while at(i).is_some_and(char::is_whitespace) {
                    i += 1;
                }
                if let Some(q) = at(i)
                    && (q == '"' || q == '\'')
                {
                    i += 1;
                    let vs = i;
                    while at(i).is_some_and(|c| c != q) {
                        i += 1;
                    }
                    attrs.push((key, text(vs, i)));
                    i += 1;
                }
            }
        }
        i += 1;
        let name = name.rsplit(':').next().unwrap_or("").to_string();
        out.push(Tag { name, attrs, open: !closing, close: closing || selfclose });
    }
    out
}

fn color(s: &str, current: [f32; 3]) -> Option<Option<[f32; 3]>> {
    let s = s.trim();
    if s == "none" || s == "transparent" {
        return Some(None);
    }
    if s == "currentColor" {
        return Some(Some(current));
    }
    if let Some(h) = s.strip_prefix('#') {
        let v: Vec<u32> = h.chars().filter_map(|c| c.to_digit(16)).collect();
        return match *v.as_slice() {
            [r, g, b] => Some(Some([r as f32 * 17.0 / 255.0, g as f32 * 17.0 / 255.0, b as f32 * 17.0 / 255.0])),
            [r0, r1, g0, g1, b0, b1] | [r0, r1, g0, g1, b0, b1, _, _] => {
                Some(Some([(r0 * 16 + r1) as f32 / 255.0, (g0 * 16 + g1) as f32 / 255.0, (b0 * 16 + b1) as f32 / 255.0]))
            }
            _ => None,
        };
    }
    if let Some(r) = s.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let v: Vec<f32> = r
            .split([',', ' '])
            .filter(|t| !t.is_empty())
            .filter_map(|t| {
                if let Some(p) = t.strip_suffix('%') { p.parse::<f32>().ok().map(|x| x / 100.0) } else { t.parse::<f32>().ok().map(|x| x / 255.0) }
            })
            .collect();
        return if let [r, g, b, ..] = *v.as_slice() { Some(Some([r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)])) } else { None };
    }
    let named = match s {
        "black" => [0.0, 0.0, 0.0],
        "white" => [1.0, 1.0, 1.0],
        "red" => [1.0, 0.0, 0.0],
        "green" => [0.0, 0.502, 0.0],
        "lime" => [0.0, 1.0, 0.0],
        "blue" => [0.0, 0.0, 1.0],
        "yellow" => [1.0, 1.0, 0.0],
        "cyan" | "aqua" => [0.0, 1.0, 1.0],
        "magenta" | "fuchsia" => [1.0, 0.0, 1.0],
        "gray" | "grey" => [0.502, 0.502, 0.502],
        "silver" => [0.753, 0.753, 0.753],
        "orange" => [1.0, 0.647, 0.0],
        "purple" => [0.502, 0.0, 0.502],
        "brown" => [0.647, 0.165, 0.165],
        "pink" => [1.0, 0.753, 0.796],
        "navy" => [0.0, 0.0, 0.502],
        _ => return if s.starts_with("url(") { Some(Some([0.5; 3])) } else { None },
    };
    Some(Some(named))
}

fn num_attr(s: &str) -> Option<f32> {
    let t: String = s.trim().chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')).collect();
    t.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn parse_transform(s: &str) -> Xf {
    let mut xf = Xf::ID;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest.get(..open).unwrap_or("").trim().trim_start_matches(',').trim();
        let Some(close) = rest.get(open..).and_then(|r| r.find(')')) else { break };
        let args: Vec<f32> = rest
            .get(open + 1..open + close)
            .unwrap_or("")
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse().ok())
            .collect();
        let t = match (name, args.as_slice()) {
            ("translate", &[x]) => Xf::translate(x, 0.0),
            ("translate", &[x, y]) => Xf::translate(x, y),
            ("scale", &[k]) => Xf::scale(k, k),
            ("scale", &[x, y]) => Xf::scale(x, y),
            ("rotate", &[a]) => Xf::rotate(a.to_radians()),
            ("rotate", &[a, x, y]) => Xf::translate(-x, -y).then(Xf::rotate(a.to_radians())).then(Xf::translate(x, y)),
            ("skewX", &[a]) => Xf([1.0, 0.0, math::tan(a.to_radians()), 1.0, 0.0, 0.0]),
            ("skewY", &[a]) => Xf([1.0, math::tan(a.to_radians()), 0.0, 1.0, 0.0, 0.0]),
            ("matrix", &[a, b, c, d, e, f]) => Xf([a, b, c, d, e, f]),
            _ => Xf::ID,
        };
        // later transforms in the list apply first (they are closer to the element)
        xf = t.then(xf);
        rest = rest.get(open + close + 1..).unwrap_or("");
    }
    xf
}

fn apply_style(st: &mut Style, key: &str, val: &str) {
    let v = val.trim();
    match key {
        "color" => {
            if let Some(Some(c)) = color(v, st.color) {
                st.color = c;
            }
        }
        "fill" => {
            if let Some(c) = color(v, st.color) {
                st.fill = c;
            }
        }
        "stroke" => {
            if let Some(c) = color(v, st.color) {
                st.stroke = c;
            }
        }
        "stroke-width" => {
            if let Some(w) = num_attr(v) {
                st.stroke_w = w.max(0.0);
            }
        }
        "opacity" => {
            if let Some(o) = num_attr(v) {
                st.opacity = o.clamp(0.0, 1.0);
            }
        }
        "fill-opacity" => {
            if let Some(o) = num_attr(v) {
                st.fill_op = o.clamp(0.0, 1.0);
            }
        }
        "stroke-opacity" => {
            if let Some(o) = num_attr(v) {
                st.stroke_op = o.clamp(0.0, 1.0);
            }
        }
        "fill-rule" => st.even_odd = v == "evenodd",
        _ => {}
    }
}

fn shape_segs(t: &Tag) -> Vec<Seg> {
    let a = |k: &str| t.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let f = |k: &str| a(k).and_then(num_attr).unwrap_or(0.0);
    match t.name.as_str() {
        "rect" => {
            let (x, y, w, h) = (f("x"), f("y"), f("width"), f("height"));
            if w <= 0.0 || h <= 0.0 {
                return vec![];
            }
            let mut rx = a("rx").and_then(num_attr);
            let mut ry = a("ry").and_then(num_attr);
            if rx.is_none() {
                rx = ry;
            }
            if ry.is_none() {
                ry = rx;
            }
            let (rx, ry) = (rx.unwrap_or(0.0).clamp(0.0, w / 2.0), ry.unwrap_or(0.0).clamp(0.0, h / 2.0));
            if rx <= 0.0 || ry <= 0.0 {
                vec![Seg::Move([x, y]), Seg::Line([x + w, y]), Seg::Line([x + w, y + h]), Seg::Line([x, y + h]), Seg::Close]
            } else {
                let mut s = vec![Seg::Move([x + rx, y]), Seg::Line([x + w - rx, y])];
                arc(&mut s, [x + w - rx, y], rx, ry, 0.0, false, true, [x + w, y + ry]);
                s.push(Seg::Line([x + w, y + h - ry]));
                arc(&mut s, [x + w, y + h - ry], rx, ry, 0.0, false, true, [x + w - rx, y + h]);
                s.push(Seg::Line([x + rx, y + h]));
                arc(&mut s, [x + rx, y + h], rx, ry, 0.0, false, true, [x, y + h - ry]);
                s.push(Seg::Line([x, y + ry]));
                arc(&mut s, [x, y + ry], rx, ry, 0.0, false, true, [x + rx, y]);
                s.push(Seg::Close);
                s
            }
        }
        "circle" | "ellipse" => {
            let (cx, cy) = (f("cx"), f("cy"));
            let (rx, ry) = if t.name == "circle" { (f("r"), f("r")) } else { (f("rx"), f("ry")) };
            if rx <= 0.0 || ry <= 0.0 {
                return vec![];
            }
            let mut s = vec![Seg::Move([cx + rx, cy])];
            arc(&mut s, [cx + rx, cy], rx, ry, 0.0, false, true, [cx, cy + ry]);
            arc(&mut s, [cx, cy + ry], rx, ry, 0.0, false, true, [cx - rx, cy]);
            arc(&mut s, [cx - rx, cy], rx, ry, 0.0, false, true, [cx, cy - ry]);
            arc(&mut s, [cx, cy - ry], rx, ry, 0.0, false, true, [cx + rx, cy]);
            s.push(Seg::Close);
            s
        }
        "line" => vec![Seg::Move([f("x1"), f("y1")]), Seg::Line([f("x2"), f("y2")])],
        "polyline" | "polygon" => {
            let v: Vec<f32> = a("points")
                .unwrap_or("")
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|t| !t.is_empty())
                .filter_map(|t| t.parse().ok())
                .collect();
            let mut s = Vec::new();
            for (i, &[x, y]) in v.as_chunks::<2>().0.iter().enumerate() {
                s.push(if i == 0 { Seg::Move([x, y]) } else { Seg::Line([x, y]) });
            }
            if t.name == "polygon" && !s.is_empty() {
                s.push(Seg::Close);
            }
            s
        }
        "path" => parse_path(a("d").unwrap_or("")),
        _ => vec![],
    }
}

/// Rasterise an SVG document into a `w` by `h` image with straight (non-premultiplied) alpha.
/// `fit`: 0 contain (keep the aspect ratio, centred), 1 stretch, 2 cover. `scale`, `dx`, `dy` (in image fractions) adjust it.
pub fn render_svg(src: &str, w: usize, h: usize, fit: u32, scale: f32, dx: f32, dy: f32) -> Result<Tex, String> {
    let ts = tags(src);
    let root = ts.iter().find(|t| t.name == "svg" && t.open).ok_or("not an SVG document (no <svg> element)")?;
    let a = |k: &str| root.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let vb: Vec<f32> = a("viewBox")
        .map(|v| v.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect())
        .unwrap_or_default();
    let (vx, vy, vw, vh) = if let [x0, y0, w0, h0] = *vb.as_slice()
        && w0 > 0.0
        && h0 > 0.0
    {
        (x0, y0, w0, h0)
    } else {
        (0.0, 0.0, a("width").and_then(num_attr).unwrap_or(100.0).max(1.0), a("height").and_then(num_attr).unwrap_or(100.0).max(1.0))
    };
    let (sx, sy) = (w as f32 / vw, h as f32 / vh);
    let (kx, ky) = match fit {
        1 => (sx, sy),
        2 => (sx.max(sy), sx.max(sy)),
        _ => (sx.min(sy), sx.min(sy)),
    };
    let (kx, ky) = (kx * scale, ky * scale);
    let base =
        Xf::translate(-vx - vw * 0.5, -vy - vh * 0.5).then(Xf::scale(kx, ky)).then(Xf::translate(w as f32 * (0.5 + dx), h as f32 * (0.5 + dy)));
    // premultiplied accumulation
    let mut acc = vec![[0.0f32; 4]; w * h];
    let mut stack: Vec<(Style, Xf)> = vec![(Style::default(), base)];
    let mut skip_depth = 0usize;
    let mut depth_names: Vec<bool> = Vec::new();
    for t in &ts {
        let hidden = matches!(
            t.name.as_str(),
            "defs"
                | "clipPath"
                | "mask"
                | "symbol"
                | "marker"
                | "pattern"
                | "linearGradient"
                | "radialGradient"
                | "style"
                | "title"
                | "desc"
                | "metadata"
                | "text"
                | "filter"
        );
        if t.open {
            if skip_depth > 0 {
                if !t.close {
                    skip_depth += 1;
                }
                continue;
            }
            if hidden {
                if !t.close {
                    skip_depth = 1;
                }
                continue;
            }
            // the stack keeps its base entry: closing tags never pop it
            let Some((mut st, mut xf)) = stack.last().cloned() else { continue };
            apply_tag_style(t, &mut st, &mut xf);
            let segs = shape_segs(t);
            if !segs.is_empty() {
                let polys = flatten(&segs, &xf);
                if let Some(c) = st.fill {
                    let m = fill(&polys, w, h, st.even_odd);
                    composite(&mut acc, &m, c, st.fill_op * st.opacity);
                }
                if let Some(c) = st.stroke
                    && st.stroke_w > 0.0
                {
                    let outline = stroke(&polys, st.stroke_w * xf.mean_scale());
                    let m = fill(&outline, w, h, false);
                    composite(&mut acc, &m, c, st.stroke_op * st.opacity);
                }
            }
            if !t.close {
                stack.push((st, xf));
                depth_names.push(true);
            }
        } else if t.close && !t.open {
            if skip_depth > 0 {
                skip_depth -= 1;
                continue;
            }
            if depth_names.pop().is_some() && stack.len() > 1 {
                stack.pop();
            }
        }
    }
    let mut out = Tex::solid(w, h, [0.0, 0.0, 0.0, 0.0]);
    for (o, p) in out.px.iter_mut().zip(&acc) {
        if p[3] > 1e-6 {
            *o = [p[0] / p[3], p[1] / p[3], p[2] / p[3], p[3].min(1.0)];
        }
    }
    Ok(out)
}

fn apply_tag_style(t: &Tag, st: &mut Style, xf: &mut Xf) {
    for (k, v) in &t.attrs {
        if k == "style" {
            for decl in v.split(';') {
                if let Some((kk, vv)) = decl.split_once(':') {
                    apply_style(st, kk.trim(), vv);
                }
            }
        } else if k == "transform" {
            *xf = parse_transform(v).then(*xf);
        } else {
            apply_style(st, k, v);
        }
    }
    // style="" must win over presentation attributes: apply it again last
    if let Some((_, v)) = t.attrs.iter().find(|(k, _)| k == "style") {
        for decl in v.split(';') {
            if let Some((kk, vv)) = decl.split_once(':') {
                apply_style(st, kk.trim(), vv);
            }
        }
    }
}

fn composite(acc: &mut [[f32; 4]], m: &Mask, c: [f32; 3], opacity: f32) {
    for (d, &cov) in acc.iter_mut().zip(&m.a) {
        let a = cov * opacity;
        if a <= 0.0 {
            continue;
        }
        let k = 1.0 - a;
        d[0] = c[0] * a + d[0] * k;
        d[1] = c[1] * a + d[1] * k;
        d[2] = c[2] * a + d[2] * k;
        d[3] = a + d[3] * k;
    }
}
