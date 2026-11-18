//! SVG path data as neutral line, cubic and elliptical arc segments.
//!
//! Relative commands and smooth controls are resolved, while arcs remain arcs.
//! The caller chooses how to map the result to its own geometry model.
use std::ops::{Add, Mul, Sub};

/// A point in the path's original user coordinate system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}
impl Point {
    /// The origin.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    /// Construct a point.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}
impl Add for Point {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}
impl Sub for Point {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}
impl Mul<f64> for Point {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

/// Invalid or oversized path data.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
type Result<T> = std::result::Result<T, Error>;
const MAX_PATH_SEGMENTS: usize = 100_000;
const MAX_CURVE_POINTS: usize = 100_001;

pub(crate) struct Numbers<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Numbers<'a> {
    pub(crate) fn new(s: &'a str) -> Self {
        Self { s: s.as_bytes(), i: 0 }
    }
    fn skip_seps(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b'\t' | b'\r' | b'\n' | b',')) {
            self.i += 1;
        }
    }

    fn next_num(&mut self) -> Option<f64> {
        self.skip_seps();
        let start = self.i;
        if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
            self.i += 1;
        }
        let mut digits = 0;
        let mut dot = false;
        let mut exp = false;
        while let Some(c) = self.s.get(self.i) {
            match c {
                b'0'..=b'9' => {
                    digits += 1;
                    self.i += 1;
                }
                b'.' if !dot && !exp => {
                    dot = true;
                    self.i += 1;
                }
                b'e' | b'E' if !exp && digits > 0 => {
                    exp = true;
                    self.i += 1;
                    if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
        if digits == 0 {
            self.i = start;
            return None;
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse::<f64>().ok().filter(|v| v.is_finite())
    }
}

impl Iterator for Numbers<'_> {
    type Item = f64;
    fn next(&mut self) -> Option<f64> {
        self.next_num()
    }
}

/// One `M …` run of a `d` attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct Subpath {
    /// Start point.
    pub start: Point,
    /// Segments in drawing order.
    pub segs: Vec<Segment>,
    /// Whether a close-path command closed this subpath.
    pub closed: bool,
}

/// A path segment; arc values retain the SVG endpoint parameterization.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Line(Point),
    Cubic(Point, Point, Point),
    Arc { rx: f64, ry: f64, rot: f64, large: bool, sweep: bool, end: Point },
}

/// Tokens of a `d` attribute: command letters and numbers, in order.
enum Tok {
    Cmd(u8),
    Num(f64),
}

struct Tokenizer<'a> {
    s: &'a [u8],
    i: usize,
    /// Start of the most recently returned token.
    tok_start: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(s: &'a str) -> Self {
        Tokenizer { s: s.as_bytes(), i: 0, tok_start: 0 }
    }

    /// Read an arc flag: a single `0` or `1` character, which needs no separator after it.
    fn next_flag(&mut self) -> Option<f64> {
        let mut nums = Numbers { s: self.s, i: self.i };
        nums.skip_seps();
        let v = match self.s.get(nums.i)? {
            b'0' => 0.0,
            b'1' => 1.0,
            _ => return None,
        };
        self.i = nums.i + 1;
        Some(v)
    }

    fn next_tok(&mut self) -> Result<Option<Tok>> {
        let mut nums = Numbers { s: self.s, i: self.i };
        nums.skip_seps();
        self.i = nums.i;
        self.tok_start = self.i;
        let Some(c) = self.s.get(self.i).copied() else { return Ok(None) };
        if c.is_ascii_alphabetic() {
            self.i += 1;
            return Ok(Some(Tok::Cmd(c)));
        }
        let tok = nums.next_num().map(Tok::Num);
        self.i = nums.i;
        // Invalid bytes and malformed/non-finite numbers are not EOF: returning a
        // successful prefix here would silently discard the rest of the shape.
        tok.map(Some).ok_or_else(|| Error("SVG: invalid path token".into()))
    }
}

/// Split `d` into subpaths, resolving relative commands and the S/T control reflections.
pub fn parse_path(d: &str) -> Result<Vec<Subpath>> {
    let mut tk = Tokenizer::new(d);
    let mut lookahead: Option<Tok> = None;
    let mut cmd = 0u8;
    let mut cur = Point::ZERO;
    let mut start = Point::ZERO;
    let mut segs: Vec<Segment> = Vec::new();
    let mut out: Vec<Subpath> = Vec::new();
    let closed = false;
    let mut prev_cubic: Option<Point> = None;
    let mut prev_quad: Option<Point> = None;
    let mut segments = 0usize;

    let next_num = |tk: &mut Tokenizer, look: &mut Option<Tok>| -> Result<f64> {
        match look.take() {
            Some(Tok::Num(v)) => Ok(v),
            Some(other @ Tok::Cmd(_)) => {
                *look = Some(other);
                Err(Error("SVG: path command is missing coordinates".into()))
            }
            None => match tk.next_tok()? {
                Some(Tok::Num(v)) => Ok(v),
                Some(other @ Tok::Cmd(_)) => {
                    *look = Some(other);
                    Err(Error("SVG: path command is missing coordinates".into()))
                }
                None => Err(Error("SVG: path command is missing coordinates".into())),
            },
        }
    };

    while let Some(tok) = match lookahead.take() {
        Some(tok) => Some(tok),
        None => tk.next_tok()?,
    } {
        let rel = match tok {
            Tok::Cmd(c) => {
                if c.eq_ignore_ascii_case(&b'z') {
                    if !segs.is_empty() {
                        out.push(Subpath { start, segs, closed: true });
                        segs = Vec::new();
                    }
                    cur = start;
                    cmd = 0;
                    // Z is not a cubic or quadratic, so a following S or T starts from the
                    // current point rather than reflecting the pre-close control.
                    prev_cubic = None;
                    prev_quad = None;
                    continue;
                }
                cmd = c;
                c.is_ascii_lowercase()
            }
            tok @ Tok::Num(_) => {
                if cmd == 0 {
                    return Err(Error("SVG: path data does not start with a command".into()));
                }
                lookahead = Some(tok);
                cmd.is_ascii_lowercase()
            }
        };
        let abs = cmd.to_ascii_uppercase();
        let count = match abs {
            b'M' | b'L' | b'T' => 2,
            b'H' | b'V' => 1,
            b'C' => 6,
            b'S' | b'Q' => 4,
            b'A' => 7,
            _ => return Err(Error(format!("SVG: unknown path command `{}`", cmd as char))),
        };
        let mut args = [0.0f64; 7];
        for (idx, slot) in args.iter_mut().enumerate().take(count) {
            if abs == b'A' && (idx == 3 || idx == 4) {
                // Flags are single characters; re-scan a pending number token from its start.
                if matches!(lookahead, Some(Tok::Num(_))) {
                    lookahead = None;
                    tk.i = tk.tok_start;
                }
                if lookahead.is_none() {
                    *slot = tk.next_flag().ok_or_else(|| Error("SVG: path command is missing coordinates".into()))?;
                    continue;
                }
            }
            *slot = next_num(&mut tk, &mut lookahead)?;
        }
        let point = |x: f64, y: f64| if rel { cur + Point::new(x, y) } else { Point::new(x, y) };
        match abs {
            b'M' => {
                if !segs.is_empty() {
                    out.push(Subpath { start, segs, closed: false });
                    segs = Vec::new();
                }
                cur = point(args[0], args[1]);
                start = cur;
                cmd = if rel { b'l' } else { b'L' };
                prev_cubic = None;
                prev_quad = None;
            }
            b'L' => {
                cur = point(args[0], args[1]);
                segs.push(Segment::Line(cur));
                prev_cubic = None;
                prev_quad = None;
            }
            b'H' => {
                cur = Point::new(if rel { cur.x + args[0] } else { args[0] }, cur.y);
                segs.push(Segment::Line(cur));
                prev_cubic = None;
                prev_quad = None;
            }
            b'V' => {
                cur = Point::new(cur.x, if rel { cur.y + args[0] } else { args[0] });
                segs.push(Segment::Line(cur));
                prev_cubic = None;
                prev_quad = None;
            }
            b'C' => {
                let (c1, c2, p) = (point(args[0], args[1]), point(args[2], args[3]), point(args[4], args[5]));
                segs.push(Segment::Cubic(c1, c2, p));
                prev_cubic = Some(c2);
                prev_quad = None;
                cur = p;
            }
            b'S' => {
                let c1 = prev_cubic.map_or(cur, |c2| cur * 2.0 - c2);
                let (c2, p) = (point(args[0], args[1]), point(args[2], args[3]));
                segs.push(Segment::Cubic(c1, c2, p));
                prev_cubic = Some(c2);
                prev_quad = None;
                cur = p;
            }
            b'Q' => {
                let (q, p) = (point(args[0], args[1]), point(args[2], args[3]));
                segs.push(Segment::Cubic(cur + (q - cur) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p));
                prev_quad = Some(q);
                prev_cubic = None;
                cur = p;
            }
            b'T' => {
                let reflect = prev_quad.map_or(cur, |q| cur * 2.0 - q);
                let p = point(args[0], args[1]);
                segs.push(Segment::Cubic(cur + (reflect - cur) * (2.0 / 3.0), p + (reflect - p) * (2.0 / 3.0), p));
                prev_quad = Some(reflect);
                prev_cubic = None;
                cur = p;
            }
            b'A' => {
                let end = point(args[5], args[6]);
                segs.push(Segment::Arc { rx: args[0].abs(), ry: args[1].abs(), rot: args[2], large: args[3] != 0.0, sweep: args[4] != 0.0, end });
                cur = end;
                prev_cubic = None;
                prev_quad = None;
            }
            _ => return Err(Error("SVG: unsupported path command".into())),
        }
        segments += 1;
        if segments > MAX_PATH_SEGMENTS {
            return Err(Error("SVG: path has too many segments".into()));
        }
        if segs.len() > MAX_CURVE_POINTS {
            return Err(Error("SVG: path has too many segments".into()));
        }
    }
    if !segs.is_empty() {
        out.push(Subpath { start, segs, closed });
    }
    Ok(out)
}
/// The centre and signed sweep of a circular arc from `p0` to `p1` in a Y-up frame, per the
/// SVG endpoint parameterisation (F.6.5). Radii that cannot span the chord are scaled up,
/// as the specification requires.
pub fn arc_center_circular(p0: Point, p1: Point, r: f64, large: bool, sweep: bool) -> Option<(Point, f64)> {
    if r <= 0.0 || !r.is_finite() {
        return None;
    }
    let d = p1 - p0;
    let dist = d.x.hypot(d.y);
    let r = if dist > 2.0 * r { dist / 2.0 } else { r };
    if dist <= f64::EPSILON {
        return None;
    }
    let mid = (p0 + p1) * 0.5;
    let h = (r * r - (dist * dist) / 4.0).max(0.0).sqrt();
    let n = Point::new(-d.y / dist, d.x / dist);
    let sign = if large != sweep { 1.0 } else { -1.0 };
    let center = mid + n * (h * sign);
    let a0 = (p0.y - center.y).atan2(p0.x - center.x);
    let a1 = (p1.y - center.y).atan2(p1.x - center.x);
    let ccw = (a1 - a0).rem_euclid(std::f64::consts::TAU);
    Some((center, pick_sweep(ccw, large, sweep)))
}

/// The signed sweep (CCW positive) for a chord sweep `ccw` in `[0, 2π)`: the centre is
/// already chosen so that the magnitude matches `large`, so only the direction is left.
fn pick_sweep(ccw: f64, large: bool, sweep: bool) -> f64 {
    let tau = std::f64::consts::TAU;
    let eps = f64::EPSILON;
    if sweep {
        if ccw <= eps { if large { tau } else { eps } } else { ccw }
    } else if ccw >= tau - eps {
        if large { -tau } else { -eps }
    } else {
        ccw - tau
    }
}

/// Endpoint → centre parameterisation for a rotated elliptical arc (SVG F.6.5), in the
/// frame the points are given in. Returns the centre, the start parameter and the signed
/// sweep.
pub fn elliptical_params(p0: Point, p1: Point, rx: f64, ry: f64, phi: f64, large: bool, sweep: bool) -> Option<(Point, f64, f64)> {
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }
    let (sphi, cphi) = phi.sin_cos();
    let dx = (p0.x - p1.x) / 2.0;
    let dy = (p0.y - p1.y) / 2.0;
    let x1 = cphi * dx + sphi * dy;
    let y1 = -sphi * dx + cphi * dy;
    let (rx, ry) = {
        let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
        if lambda > 1.0 {
            let s = lambda.sqrt();
            (rx * s, ry * s)
        } else {
            (rx, ry)
        }
    };
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let coef = if sweep != large { 1.0 } else { -1.0 } * (num.max(0.0) / den.max(f64::MIN_POSITIVE)).sqrt();
    let cxp = coef * rx * y1 / ry;
    let cyp = -coef * ry * x1 / rx;
    let center = Point::new(cphi * cxp - sphi * cyp + (p0.x + p1.x) / 2.0, sphi * cxp + cphi * cyp + (p0.y + p1.y) / 2.0);
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let dot = ux * vx + uy * vy;
        let det = ux * vy - uy * vx;
        let denom = (ux.hypot(uy) * vx.hypot(vy)).max(f64::MIN_POSITIVE);
        (dot / denom).clamp(-1.0, 1.0).acos().copysign(det)
    };
    let (ux, uy) = ((x1 - cxp) / rx, (y1 - cyp) / ry);
    let (vx, vy) = ((-x1 - cxp) / rx, (-y1 - cyp) / ry);
    let theta = angle(1.0, 0.0, ux, uy);
    let ccw = angle(ux, uy, vx, vy).rem_euclid(std::f64::consts::TAU);
    Some((center, theta, pick_sweep(ccw, large, sweep)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn resolves_relative_and_smooth_commands_without_flattening_arcs() {
        let paths = parse_path("M1 2 l3 4 C5 6 7 8 9 10 s2 3 4 5 A6 7 30 0110 12 z").unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].closed);
        assert_eq!(paths[0].start, Point::new(1.0, 2.0));
        assert_eq!(paths[0].segs[0], Segment::Line(Point::new(4.0, 6.0)));
        assert_eq!(paths[0].segs[2], Segment::Cubic(Point::new(11.0, 12.0), Point::new(11.0, 13.0), Point::new(13.0, 15.0)));
        assert_eq!(paths[0].segs[3], Segment::Arc { rx: 6.0, ry: 7.0, rot: 30.0, large: false, sweep: true, end: Point::new(10.0, 12.0) });
    }

    #[test]
    fn smooth_quadratic_consumes_both_coordinates() {
        let paths = parse_path("M0 0 Q3 3 6 0 T12 0").unwrap();
        assert_eq!(paths[0].segs[1], Segment::Cubic(Point::new(8.0, -2.0), Point::new(10.0, -2.0), Point::new(12.0, 0.0)));
    }

    #[test]
    fn close_resets_smooth_control_history() {
        // After Z, S must not reflect the preceding C's control (20,20) → (0,0).
        let paths = parse_path("M10 10 C10 20 20 20 20 10 Z S30 0 30 10").unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].closed);
        assert_eq!(paths[1].segs[0], Segment::Cubic(Point::new(10.0, 10.0), Point::new(30.0, 0.0), Point::new(30.0, 10.0)));
    }

    #[test]
    fn arc_centers_keep_radius_correction_and_sweep() {
        let (center, delta) = arc_center_circular(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 2.0, false, true).unwrap();
        assert!((center.x - 5.0).abs() < 1e-10);
        assert!(center.y.abs() < 1e-10);
        assert!((delta - std::f64::consts::PI).abs() < 1e-10);
        let (center, start, sweep) = elliptical_params(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 2.0, 1.0, 0.0, false, true).unwrap();
        assert!((center.x - 5.0).abs() < 1e-10);
        assert!(center.y.abs() < 1e-10 && start.is_finite());
        assert!((sweep - std::f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn rejects_invalid_tokens_instead_of_returning_a_partial_path() {
        for input in ["@", "M0 0 L1 1@ L2 2", "M0 0 L1 1@", "M0 0 Z@", "M0 0 L1 1é", "M0 0 L1 1 +", "M0 0 L1 1 .", "M0 0 L1 1 2e+", "M0 0 L1 1 1e999"]
        {
            assert!(parse_path(input).is_err(), "accepted malformed path: {input}");
        }
    }

    #[test]
    fn token_errors_do_not_change_clean_eof_or_compact_numbers() {
        assert!(parse_path(" \t\r\n,").unwrap().is_empty());
        let paths = parse_path("M.5-.5 L1e1-2e0 A6 7 30 0110 12 z ,\n").unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].closed);
        assert_eq!(paths[0].start, Point::new(0.5, -0.5));
        assert_eq!(paths[0].segs[0], Segment::Line(Point::new(10.0, -2.0)));
        assert_eq!(paths[0].segs[1], Segment::Arc { rx: 6.0, ry: 7.0, rot: 30.0, large: false, sweep: true, end: Point::new(10.0, 12.0) });
    }

    #[test]
    fn rejects_invalid_flags_and_limits_segments() {
        assert!(parse_path("M0 0 A1 1 0 5 1 2 2").is_err());
        assert!(parse_path("M0 0 L").is_err());
        let excessive = format!("M0 0{}", " L1 1".repeat(MAX_PATH_SEGMENTS));
        assert!(parse_path(&excessive).is_err());
    }
}
