// SPDX-License-Identifier: MIT OR Apache-2.0
//! Tone curves: a list of control points joined by smooth cubic segments, baked into a lookup table.
//!
//! Text form (used by the `curve` node): `x:y` points separated by commas, e.g. `0:0,0.5:0.7,1:1`.
//! A point may carry a slope (`x:y:s`, the same slope on both sides) or two (`x:y:sin:sout`).
//! Points without a slope get a smooth, overshoot-free slope from their neighbours.

use crate::tex::Tex;

#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    /// (x, y, slope in, slope out), sorted by x
    pub pts: Vec<(f32, f32, f32, f32)>,
}

pub const LUT: usize = 1024;

impl Curve {
    pub fn identity() -> Curve {
        Curve::new(vec![(0.0, 0.0, None, None), (1.0, 1.0, None, None)])
    }

    /// Points with optional explicit slopes; computes the automatic ones.
    pub fn new(mut raw: Vec<(f32, f32, Option<f32>, Option<f32>)>) -> Curve {
        raw.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        raw.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6);
        if raw.is_empty() {
            return Curve::identity();
        }
        if let [p] = raw.as_slice() {
            let y = p.1;
            raw = vec![(0.0, y, None, None), (1.0, y, None, None)];
        }
        let n = raw.len();
        // secant slopes and Fritsch-Carlson monotone tangents
        let sec: Vec<f32> = raw
            .windows(2)
            .map(|w| match w {
                [a, b] => (b.1 - a.1) / (b.0 - a.0),
                _ => 0.0,
            })
            .collect();
        let mut auto = vec![0.0f32; n];
        if let (Some(a), Some(&s)) = (auto.first_mut(), sec.first()) {
            *a = s;
        }
        if let (Some(a), Some(&s)) = (auto.last_mut(), sec.last()) {
            *a = s;
        }
        for (a, w) in auto.iter_mut().skip(1).zip(sec.windows(2)) {
            if let [s0, s1] = *w {
                *a = if s0 * s1 <= 0.0 { 0.0 } else { (s0 + s1) * 0.5 };
            }
        }
        for (i, &si) in sec.iter().enumerate() {
            let (Some(&ai), Some(&ai1)) = (auto.get(i), auto.get(i + 1)) else { continue };
            let (ni, ni1) = if si == 0.0 {
                (0.0, 0.0)
            } else {
                let (a, b) = (ai / si, ai1 / si);
                let s = a * a + b * b;
                if s > 9.0 {
                    let t = 3.0 / s.sqrt();
                    (t * a * si, t * b * si)
                } else {
                    (ai, ai1)
                }
            };
            if let Some(v) = auto.get_mut(i) {
                *v = ni;
            }
            if let Some(v) = auto.get_mut(i + 1) {
                *v = ni1;
            }
        }
        let pts = raw
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ai = auto.get(i).copied().unwrap_or(0.0);
                let s_in = p.2.unwrap_or(ai);
                let s_out = p.3.or(p.2).unwrap_or(ai);
                (p.0.clamp(0.0, 1.0), p.1.clamp(0.0, 1.0), s_in, s_out)
            })
            .collect();
        Curve { pts }
    }

    pub fn parse(text: &str) -> Result<Curve, String> {
        let mut raw = Vec::new();
        for item in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let parts: Vec<&str> = item.split(':').collect();
            let num =
                |s: &str| s.trim().parse::<f32>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("bad number '{s}' in curve point '{item}'"));
            match *parts.as_slice() {
                [x, y] => raw.push((num(x)?, num(y)?, None, None)),
                [x, y, s] => raw.push((num(x)?, num(y)?, Some(num(s)?), None)),
                [x, y, si, so] => raw.push((num(x)?, num(y)?, Some(num(si)?), Some(num(so)?))),
                _ => return Err(format!("curve point '{item}' must look like x:y")),
            }
        }
        if raw.is_empty() {
            return Err("a curve needs at least one point".into());
        }
        Ok(Curve::new(raw))
    }

    pub fn to_text(&self) -> String {
        self.pts.iter().map(|p| format!("{}:{}:{}:{}", p.0, p.1, p.2, p.3)).collect::<Vec<_>>().join(",")
    }

    pub fn eval(&self, x: f32) -> f32 {
        let p = &self.pts;
        let (Some(&first), Some(&last)) = (p.first(), p.last()) else { return 0.0 };
        if x <= first.0 {
            return first.1;
        }
        if x >= last.0 {
            return last.1;
        }
        let i = p.partition_point(|q| q.0 <= x).saturating_sub(1);
        let (Some(&a), Some(&b)) = (p.get(i), p.get(i + 1)) else { return last.1 };
        let h = b.0 - a.0;
        let t = (x - a.0) / h;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * a.1 + (t3 - 2.0 * t2 + t) * h * a.3 + (-2.0 * t3 + 3.0 * t2) * b.1 + (t3 - t2) * h * b.2;
        y.clamp(0.0, 1.0)
    }

    pub fn lut(&self) -> Vec<f32> {
        (0..LUT).map(|i| self.eval(i as f32 / (LUT - 1) as f32)).collect()
    }
}

fn lookup(lut: &[f32], x: f32) -> f32 {
    let f = x.clamp(0.0, 1.0) * (LUT - 1) as f32;
    let i = (f as usize).min(LUT - 2);
    let t = f - i as f32;
    let (a, b) = (lut.get(i).copied().unwrap_or(0.0), lut.get(i + 1).copied().unwrap_or(0.0));
    a + (b - a) * t
}

/// Apply the master curve to R, G and B, then the per-channel curves (r, g, b, a).
pub fn apply(t: &Tex, master: &Curve, ch: [&Curve; 4]) -> Tex {
    let m = master.lut();
    let l: Vec<Vec<f32>> = ch.iter().map(|c| c.lut()).collect();
    let mut out = t.clone();
    let [l0, l1, l2, l3] = [0, 1, 2, 3].map(|k| l.get(k).map_or(&[] as &[f32], Vec::as_slice));
    for p in out.px.iter_mut() {
        p[0] = lookup(l0, lookup(&m, p[0]));
        p[1] = lookup(l1, lookup(&m, p[1]));
        p[2] = lookup(l2, lookup(&m, p[2]));
        p[3] = lookup(l3, p[3]);
    }
    out
}
