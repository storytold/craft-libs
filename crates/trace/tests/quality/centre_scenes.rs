//! Code-drawn test scenes with exact answers: centre lines with widths (round caps), filled
//! areas, junctions with their degrees, and corners (stroke ends that meet but are no junction).

use super::centre_geom::{P, seg_dist};

#[derive(Clone, Debug)]
pub struct GtStroke {
    /// Dense centre line (≤ 0.25 px apart).
    pub pts: Vec<P>,
    pub w: Vec<f64>,
    pub closed: bool,
}

#[derive(Clone, Debug)]
pub enum GtFill {
    Disc(P, f64),
    Poly(Vec<P>),
}

impl GtFill {
    pub fn inside(&self, q: P) -> bool {
        match self {
            GtFill::Disc(c, r) => q.dist(*c) <= *r,
            GtFill::Poly(p) => {
                let n = p.len();
                let mut inside = false;
                for i in 0..n {
                    let (a, b) = (p[i], p[(i + 1) % n]);
                    if (a.y > q.y) != (b.y > q.y) && q.x < a.x + (q.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                        inside = !inside;
                    }
                }
                inside
            }
        }
    }
    /// Distance outside (0 inside), approximate for polygons (to the edges).
    pub fn dist(&self, q: P) -> f64 {
        if self.inside(q) {
            return 0.0;
        }
        match self {
            GtFill::Disc(c, r) => q.dist(*c) - r,
            GtFill::Poly(p) => {
                let n = p.len();
                (0..n).map(|i| seg_dist(q, p[i], p[(i + 1) % n]).0).fold(f64::MAX, f64::min)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub name: String,
    pub w: usize,
    pub h: usize,
    pub strokes: Vec<GtStroke>,
    pub fills: Vec<GtFill>,
    pub junctions: Vec<(P, usize)>,
    pub corners: Vec<P>,
    pub wmax: f64,
    pub min_len: f64,
    /// Added to every coordinate (odd widths at integer positions put pixel centres exactly on
    /// the edge: ties that make coverage numbers meaningless).
    pub offset: P,
}

impl Scene {
    fn new(name: &str, w: usize, h: usize, wmax: f64) -> Self {
        Self {
            name: name.into(),
            w,
            h,
            strokes: vec![],
            fills: vec![],
            junctions: vec![],
            corners: vec![],
            wmax,
            min_len: 20.0,
            offset: P::new(0.0, 0.0),
        }
    }
    /// A polyline stroke of constant width.
    fn line(&mut self, pts: &[P], w: f64) {
        self.var(pts, &vec![w; pts.len()], false);
    }
    /// A polyline stroke with a width at each vertex (linear between), densified.
    fn var(&mut self, pts: &[P], ws: &[f64], closed: bool) {
        let pts: Vec<P> = pts.iter().map(|&p| p + self.offset).collect();
        let pts = &pts[..];
        let mut d = vec![];
        let mut dw = vec![];
        let n = pts.len();
        let segs = if closed { n } else { n.saturating_sub(1) };
        for i in 0..segs {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (wa, wb) = (ws[i], ws[(i + 1) % n]);
            let k = ((a.dist(b) / 0.25).ceil() as usize).max(1);
            for j in 0..k {
                let t = j as f64 / k as f64;
                d.push(a.lerp(b, t));
                dw.push(wa + (wb - wa) * t);
            }
        }
        if !closed && let (Some(&l), Some(&lw)) = (pts.last(), ws.last()) {
            d.push(l);
            dw.push(lw);
        }
        self.strokes.push(GtStroke { pts: d, w: dw, closed });
    }
    fn curve(&mut self, f: impl Fn(f64) -> P, wf: impl Fn(f64) -> f64, n: usize, closed: bool) {
        let m = if closed { n } else { n + 1 };
        let pts: Vec<P> = (0..m).map(|k| f(k as f64 / n as f64)).collect();
        let ws: Vec<f64> = (0..m).map(|k| wf(k as f64 / n as f64)).collect();
        self.var(&pts, &ws, closed);
    }
    fn arc(&mut self, c: P, r: f64, a0: f64, a1: f64, w: f64) {
        let n = ((r * (a1 - a0).abs() / 0.5).ceil() as usize).max(8);
        self.curve(
            |t| {
                let a = a0 + (a1 - a0) * t;
                c + P::new(r * a.cos(), r * a.sin())
            },
            |_| w,
            n,
            false,
        );
    }
    fn ring(&mut self, c: P, r: f64, w: f64) {
        let n = ((r * std::f64::consts::TAU / 0.5).ceil() as usize).max(16);
        self.curve(
            |t| {
                let a = std::f64::consts::TAU * t;
                c + P::new(r * a.cos(), r * a.sin())
            },
            |_| w,
            n,
            true,
        );
    }

    /// Ink coverage (8 × 8 samples per pixel).
    pub fn rasterise(&self) -> Vec<f32> {
        self.rasterise_at(8)
    }
    /// Exact shape point-sampled at pixel centres (the reference for coverage).
    pub fn exact(&self) -> Vec<bool> {
        self.rasterise_at(1).into_iter().map(|c| c >= 1.0).collect()
    }
    fn rasterise_at(&self, ss: usize) -> Vec<f32> {
        let (sw, sh) = (self.w * ss, self.h * ss);
        let mut sub = vec![false; sw * sh];
        let at = |x: usize, y: usize| P::new((x as f64 + 0.5) / ss as f64, (y as f64 + 0.5) / ss as f64);
        for s in &self.strokes {
            let n = s.pts.len();
            let segs = if s.closed { n } else { n.saturating_sub(1) };
            let mut stamp = |a: P, b: P, ra: f64, rb: f64| {
                let r = ra.max(rb);
                let x0 = (((a.x.min(b.x) - r) * ss as f64).floor().max(0.0)) as usize;
                let y0 = (((a.y.min(b.y) - r) * ss as f64).floor().max(0.0)) as usize;
                let x1 = ((((a.x.max(b.x) + r) * ss as f64).ceil().max(0.0)) as usize).min(sw);
                let y1 = ((((a.y.max(b.y) + r) * ss as f64).ceil().max(0.0)) as usize).min(sh);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let q = at(x, y);
                        let (d, t) = seg_dist(q, a, b);
                        if d <= ra + (rb - ra) * t {
                            sub[y * sw + x] = true;
                        }
                    }
                }
            };
            if n == 1 {
                stamp(s.pts[0], s.pts[0], s.w[0] / 2.0, s.w[0] / 2.0);
            }
            for i in 0..segs {
                let j = (i + 1) % n;
                stamp(s.pts[i], s.pts[j], s.w[i] / 2.0, s.w[j] / 2.0);
            }
        }
        for f in &self.fills {
            for y in 0..sh {
                for x in 0..sw {
                    if !sub[y * sw + x] && f.inside(at(x, y)) {
                        sub[y * sw + x] = true;
                    }
                }
            }
        }
        let mut out = vec![0.0f32; self.w * self.h];
        for y in 0..self.h {
            for x in 0..self.w {
                let mut c = 0;
                for yy in 0..ss {
                    for xx in 0..ss {
                        c += usize::from(sub[(y * ss + yy) * sw + x * ss + xx]);
                    }
                }
                out[y * self.w + x] = c as f32 / (ss * ss) as f32;
            }
        }
        out
    }
}

fn pol(c: P, r: f64, deg: f64) -> P {
    let a = deg.to_radians();
    c + P::new(r * a.cos(), r * a.sin())
}

/// 24 straight lines of width `w`, at 0°–172.5° in 7.5° steps, centres at varied sub-pixel phases.
pub fn lines(w: f64) -> Scene {
    let mut s = Scene::new(&format!("lines w{w}"), 480, 320, 13.0);
    for k in 0..24 {
        let (cx, cy) = ((k % 6) as f64 * 80.0 + 40.0 + 0.13 * k as f64 % 1.0, (k / 6) as f64 * 80.0 + 40.0 + 0.29 * k as f64 % 1.0);
        let c = P::new(cx, cy);
        let a = 7.5 * k as f64;
        s.line(&[pol(c, 25.0, a + 180.0), pol(c, 25.0, a)], w);
    }
    s
}

pub fn curves(w: f64) -> Scene {
    let mut s = Scene::new(&format!("curves w{w}"), 400, 300, 13.0);
    // Sine waves.
    s.curve(|t| P::new(30.0 + 340.0 * t, 50.0 + 20.0 * (t * 12.0).sin()), |_| w, 400, false);
    s.curve(|t| P::new(30.0 + 340.0 * t, 110.0 + 10.0 * (t * 25.0).sin()), |_| w, 600, false);
    // Cubic S-curve.
    let cubic = |t: f64| {
        let (p0, p1, p2, p3) = (P::new(40.0, 280.0), P::new(60.0, 140.0), P::new(200.0, 300.0), P::new(190.0, 160.0));
        let u = 1.0 - t;
        p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t)
    };
    s.curve(cubic, |_| w, 300, false);
    // Spiral.
    s.curve(
        |t| {
            let a = t * 4.0 * std::f64::consts::PI;
            P::new(300.0, 225.0) + P::new(a.cos(), a.sin()) * (12.0 + 50.0 * t)
        },
        |_| w,
        800,
        false,
    );
    s
}

pub fn circles() -> Scene {
    let mut s = Scene::new("circles", 520, 260, 13.0);
    let mut x = 50.0;
    for (r, w) in [(8.0, 1.0), (12.0, 2.0), (18.0, 3.0), (25.0, 4.0), (35.0, 6.0), (45.0, 8.0)] {
        s.ring(P::new(x, 70.0 + 0.3 * r % 1.0), r, w);
        s.ring(P::new(x + 10.0, 190.0), r * 1.2, (w * 0.5_f64).max(1.0));
        x += 2.0 * r + 35.0;
    }
    s
}

pub fn junctions(w: f64) -> Scene {
    let mut s = Scene::new(&format!("junctions w{w}"), 520, 220, 13.0);
    s.offset = P::new(0.27, 0.41);
    // T.
    s.line(&[P::new(20.0, 40.0), P::new(100.0, 40.0)], w);
    s.line(&[P::new(60.0, 40.0), P::new(60.0, 100.0)], w);
    s.junctions.push((P::new(60.0, 40.0), 3));
    // Y (120°).
    let c = P::new(180.0, 70.0);
    for a in [-90.0, 30.0, 150.0] {
        s.line(&[c, pol(c, 40.0, a)], w);
    }
    s.junctions.push((c, 3));
    // X (90°).
    let c = P::new(300.0, 70.0);
    s.line(&[pol(c, 40.0, 45.0), pol(c, 40.0, 225.0)], w);
    s.line(&[pol(c, 40.0, 135.0), pol(c, 40.0, 315.0)], w);
    s.junctions.push((c, 4));
    // Oblique T (stem at 50°).
    s.line(&[P::new(380.0, 40.0), P::new(500.0, 40.0)], w);
    s.line(&[P::new(430.0, 40.0), pol(P::new(430.0, 40.0), 70.0, 50.0)], w);
    s.junctions.push((P::new(430.0, 40.0), 3));
    // Thin stem on a thick bar.
    let bar = (w * 2.0).min(12.0);
    s.line(&[P::new(20.0, 160.0), P::new(140.0, 160.0)], bar);
    s.line(&[P::new(80.0, 160.0), P::new(80.0, 215.0)], (w * 0.75).max(1.0));
    s.junctions.push((P::new(80.0, 160.0), 3));
    // Narrow Y (arms 40° apart).
    let c = P::new(230.0, 150.0);
    s.line(&[c, pol(c, 60.0, 180.0)], w);
    s.line(&[c, pol(c, 50.0, -20.0)], w);
    s.line(&[c, pol(c, 50.0, 20.0)], w);
    s.junctions.push((c, 3));
    // Asterisk-free "K": arms meeting the stem at one point (degree 4).
    let c = P::new(400.0, 165.0);
    s.line(&[P::new(400.0, 120.0), P::new(400.0, 210.0)], w);
    s.line(&[c, pol(c, 50.0, -40.0)], w);
    s.line(&[c, pol(c, 50.0, 40.0)], w);
    s.junctions.push((c, 4));
    shifted(s)
}

pub fn crossings(w: f64) -> Scene {
    let mut s = Scene::new(&format!("crossings w{w}"), 560, 140, 13.0);
    s.offset = P::new(0.27, 0.41);
    for (k, ang) in [20.0, 30.0, 45.0, 60.0, 90.0].into_iter().enumerate() {
        let c = P::new(60.0 + 110.0 * k as f64, 70.0);
        s.line(&[pol(c, 50.0, 180.0 - ang / 2.0), pol(c, 50.0, -ang / 2.0)], w);
        s.line(&[pol(c, 50.0, 180.0 + ang / 2.0), pol(c, 50.0, ang / 2.0)], w);
        s.junctions.push((c, 4));
    }
    shifted(s)
}

pub fn dashed(w: f64) -> Scene {
    let mut s = Scene::new(&format!("dashed w{w}"), 400, 260, 13.0);
    s.min_len = 0.0;
    for (k, (dash, gap, ang)) in [(12.0, 4.0, 0.0), (6.0, 3.0, 15.0), (10.0, 4.0, 37.0), (16.0, 6.0, 90.0), (8.0, 5.0, 135.0)].into_iter().enumerate()
    {
        let c = P::new(80.0 + 120.0 * (k % 3) as f64, 70.0 + 120.0 * (k / 3) as f64);
        let a: f64 = f64::to_radians(ang);
        let dir = P::new(a.cos(), a.sin());
        let mut t = -50.0;
        while t + dash <= 50.0 {
            s.line(&[c + dir * t, c + dir * (t + dash)], w);
            // The visible gap is `gap`: the round caps reach w/2 past each dash's ends.
            t += dash + gap + w;
        }
    }
    s
}

/// Letter-like strokes drawn from lines and arcs (no fonts).
pub fn letters(w: f64) -> Scene {
    let mut s = Scene::new(&format!("letters w{w}"), 520, 120, 13.0);
    s.offset = P::new(0.27, 0.41);
    // A: two legs meeting at the apex (a corner), crossbar (two T's).
    let (l, apex, r) = (P::new(20.0, 100.0), P::new(50.0, 20.0), P::new(80.0, 100.0));
    s.line(&[l, apex, r], w);
    let (bl, br) = (l.lerp(apex, 0.4), r.lerp(apex, 0.4));
    s.line(&[bl, br], w);
    s.junctions.push((bl, 3));
    s.junctions.push((br, 3));
    s.corners.push(apex);
    // H.
    s.line(&[P::new(110.0, 20.0), P::new(110.0, 100.0)], w);
    s.line(&[P::new(160.0, 20.0), P::new(160.0, 100.0)], w);
    s.line(&[P::new(110.0, 60.0), P::new(160.0, 60.0)], w);
    s.junctions.push((P::new(110.0, 60.0), 3));
    s.junctions.push((P::new(160.0, 60.0), 3));
    // e: bar across a circle that opens at the lower right.
    let c = P::new(215.0, 65.0);
    s.arc(c, 25.0, 0.0, -std::f64::consts::PI * 1.75, w);
    s.line(&[pol(c, 25.0, 180.0), pol(c, 25.0, 0.0)], w);
    s.junctions.push((pol(c, 25.0, 180.0), 3));
    s.corners.push(pol(c, 25.0, 0.0));
    // k.
    let c = P::new(275.0, 75.0);
    s.line(&[P::new(275.0, 15.0), P::new(275.0, 105.0)], w);
    s.line(&[c, P::new(310.0, 45.0)], w);
    s.line(&[c, P::new(310.0, 105.0)], w);
    s.junctions.push((c, 4));
    // x.
    s.line(&[P::new(330.0, 40.0), P::new(370.0, 100.0)], w);
    s.line(&[P::new(370.0, 40.0), P::new(330.0, 100.0)], w);
    s.junctions.push((P::new(350.0, 70.0), 4));
    // s: two arcs.
    s.arc(P::new(405.0, 50.0), 15.0, -0.2, -std::f64::consts::PI * 1.5, w);
    s.arc(P::new(405.0, 80.0), 15.0, -std::f64::consts::PI * 0.5, std::f64::consts::PI * 0.8, w);
    s.corners.push(P::new(405.0, 65.0));
    // o.
    s.ring(P::new(475.0, 65.0), 25.0, w);
    shifted(s)
}

/// Thin strokes and wide fills, with strokes running into fills; Stroke width 6.
pub fn mix() -> Scene {
    let mut s = Scene::new("mix", 480, 300, 6.0);
    s.fills.push(GtFill::Disc(P::new(100.0, 100.0), 30.0));
    s.fills.push(GtFill::Poly(vec![P::new(260.0, 60.0), P::new(340.0, 60.0), P::new(340.0, 110.0), P::new(260.0, 110.0)]));
    s.fills.push(GtFill::Disc(P::new(400.0, 220.0), 22.0));
    // Lines into the disc and the rectangle.
    s.line(&[P::new(20.0, 200.0), P::new(80.0, 120.0)], 3.0);
    s.line(&[P::new(130.0, 100.0), P::new(260.0, 85.0)], 2.0);
    s.line(&[P::new(300.0, 110.0), P::new(300.0, 200.0), P::new(380.0, 210.0)], 4.0);
    // Free strokes.
    s.line(&[P::new(40.0, 260.0), P::new(250.0, 280.0)], 3.0);
    s.curve(|t| P::new(150.0 + 120.0 * t, 200.0 + 25.0 * (t * 7.0).sin()), |_| 2.0, 200, false);
    s.line(&[P::new(420.0, 30.0), P::new(460.0, 150.0)], 5.0);
    s
}

/// One connected shape: a wide disc with three thin arms of 2, 3 and 5 px reaching out of it,
/// and nothing else; Stroke width 6.
pub fn mixed_width_body() -> Scene {
    let mut s = Scene::new("mixed-width body", 260, 220, 6.0);
    s.offset = P::new(0.27, 0.41);
    let c = P::new(100.0, 100.0);
    s.fills.push(GtFill::Disc(c + s.offset, 24.0));
    s.line(&[c, P::new(220.0, 100.0)], 3.0);
    s.line(&[c, P::new(100.0, 20.0)], 5.0);
    s.line(&[c, P::new(25.0, 185.0)], 2.0);
    s
}

/// Strokes whose width varies: tapers, a lens, a swelling brush curve; Stroke width 12.
pub fn varying() -> Scene {
    let mut s = Scene::new("varying", 480, 300, 12.0);
    s.curve(|t| P::new(30.0 + 200.0 * t, 40.0), |t| 2.0 + 8.0 * t, 200, false);
    s.curve(|t| P::new(260.0 + 200.0 * t, 40.0 + 30.0 * t), |t| 2.0 + 8.0 * (std::f64::consts::PI * t).sin(), 200, false);
    s.curve(|t| P::new(30.0 + 420.0 * t, 150.0 + 40.0 * (t * 6.0).sin()), |t| 5.5 + 3.0 * (t * 9.0).sin(), 600, false);
    s.curve(
        |t| {
            let a = t * 1.6 * std::f64::consts::PI;
            P::new(120.0, 250.0) + P::new(a.cos(), a.sin()) * 35.0
        },
        |t| 2.0 + 9.0 * t,
        300,
        false,
    );
    s.curve(|t| P::new(250.0 + 200.0 * t, 260.0 - 50.0 * t * t), |t| 9.0 - 6.0 * t, 200, false);
    s
}

/// Shifted copies of the integer-placed scenes' junctions and corners (strokes shift in `var`).
fn shifted(mut s: Scene) -> Scene {
    let o = s.offset;
    for j in &mut s.junctions {
        j.0 = j.0 + o;
    }
    for c in &mut s.corners {
        *c = *c + o;
    }
    s
}

pub fn all() -> Vec<Scene> {
    let mut v = vec![];
    for w in [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0, 12.0] {
        v.push(lines(w));
    }
    for w in [2.0, 4.0, 8.0] {
        v.push(curves(w));
    }
    v.push(circles());
    for w in [2.0, 4.0, 8.0] {
        v.push(junctions(w));
    }
    for w in [3.0, 6.0] {
        v.push(crossings(w));
    }
    for w in [2.0, 4.0] {
        v.push(dashed(w));
    }
    for w in [3.0, 5.0] {
        v.push(letters(w));
    }
    v.push(mix());
    v.push(mixed_width_body());
    v.push(varying());
    v
}
