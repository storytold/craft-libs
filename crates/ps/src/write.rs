//! PostScript writing primitives: numbers, strings, matrices, path data and Flate.
//!
//! The caller decides the page program and any short operator names a prolog binds; this module
//! makes what it writes valid PostScript tokens. Numbers that are not finite are written as zero,
//! because PostScript has no spelling for them. Strings and byte strings are escaped so no value
//! can break out of a literal.
//!
//! ```
//! use craft_ps::write::{num, push_path, string, PathOps};
//! use kurbo::BezPath;
//!
//! assert_eq!(num(1.5), "1.5");
//! assert_eq!(string("a (b)"), "(a \\(b\\))");
//!
//! let mut path = BezPath::new();
//! path.move_to((1.0, 2.0));
//! path.line_to((3.0, 4.0));
//! path.close_path();
//! let mut out = String::new();
//! push_path(&mut out, &path, PathOps::SHORT);
//! assert_eq!(out, "1 2 m\n3 4 l\nh\n");
//! ```

use std::fmt::Write;
use std::io::{Read, Write as _};

use kurbo::{Affine, BezPath, PathEl};

/// `v` as PostScript writes it: at most four decimals, no trailing zeros (0 when not finite).
pub fn num(v: f64) -> String {
    let mut s = String::new();
    push_num(&mut s, v);
    s
}

/// Append [`num`]`(v)` to `s`.
pub fn push_num(s: &mut String, v: f64) {
    push_num_places(s, v, 4);
}

/// The value [`num`]`(v)` stands for, as a reader parses it back.
pub fn written(v: f64) -> f64 {
    num(v).parse().unwrap_or(0.0)
}

/// Append `v` to `s` with at most `places` decimals (up to 9), no trailing zeros (0 when not
/// finite).
pub fn push_num_places(s: &mut String, v: f64, places: u32) {
    let places = places.min(9);
    let scale = 10f64.powi(places as i32);
    // Past where the scaling overflows, values have no fractional digits to round.
    let v = if !v.is_finite() {
        0.0
    } else if (v * scale).is_finite() {
        (v * scale).round() / scale
    } else {
        v
    };
    if v == v.trunc() && v.abs() < 1e15 {
        let _ = write!(s, "{}", v as i64);
        return;
    }
    let start = s.len();
    let _ = write!(s, "{v:.*}", places as usize);
    // Only trim fractional zeros. With `places == 0` the token has no decimal point, and
    // trimming would turn 1e15 into "1".
    if s[start..].contains('.') {
        let keep = s[start..].trim_end_matches('0').trim_end_matches('.').len();
        s.truncate(start + keep);
    }
}

/// `vs` as numbers separated by spaces, then a space.
pub fn push_nums(s: &mut String, vs: &[f64]) {
    for v in vs {
        push_num(s, *v);
        s.push(' ');
    }
}

/// `text` as a PostScript string: `(` `)` `\` escaped, anything outside printable ASCII as octal
/// escapes of its UTF-8 bytes.
pub fn string(text: &str) -> String {
    ps_literal(text.bytes())
}

/// `bytes` as a PostScript string: `(` `)` `\` escaped, carriage returns, line feeds and tabs as
/// `\r` `\n` `\t`, other bytes outside printable ASCII as octal escapes.
pub fn bytes(bytes: &[u8]) -> String {
    ps_literal_bytes(bytes, true)
}

fn ps_literal(bytes: impl Iterator<Item = u8>) -> String {
    ps_literal_bytes_iter(bytes, false)
}

fn ps_literal_bytes(bytes: &[u8], escape_whitespace: bool) -> String {
    ps_literal_bytes_iter(bytes.iter().copied(), escape_whitespace)
}

fn ps_literal_bytes_iter(bytes: impl Iterator<Item = u8>, escape_whitespace: bool) -> String {
    let mut s = String::from("(");
    for b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(char::from(b));
            }
            b'\r' if escape_whitespace => s.push_str("\\r"),
            b'\n' if escape_whitespace => s.push_str("\\n"),
            b'\t' if escape_whitespace => s.push_str("\\t"),
            0x20..=0x7e => s.push(char::from(b)),
            _ => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s.push(')');
    s
}

/// A matrix operand `[a b c d e f]`.
pub fn matrix(a: Affine) -> String {
    let mut s = String::from("[");
    push_nums(&mut s, &a.as_coeffs());
    s.pop();
    s.push(']');
    s
}

/// Names of the path operators written after each segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathOps {
    pub moveto: &'static str,
    pub lineto: &'static str,
    pub curveto: &'static str,
    pub closepath: &'static str,
}

impl PathOps {
    /// The PostScript Language Reference names.
    pub const STANDARD: Self = Self { moveto: "moveto", lineto: "lineto", curveto: "curveto", closepath: "closepath" };
    /// One-letter names a bound prolog can define (`m`, `l`, `c`, `h`).
    pub const SHORT: Self = Self { moveto: "m", lineto: "l", curveto: "c", closepath: "h" };
}

/// Append path `path` (`moveto`, `lineto`, `curveto`, `closepath` by default; quadratics raised to
/// cubics), one segment a line. Pass [`PathOps::SHORT`] for the one-letter names a prolog binds.
pub fn push_path(s: &mut String, path: &BezPath, ops: PathOps) {
    let mut last = kurbo::Point::ZERO;
    let mut start = kurbo::Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                push_nums(s, &[p.x, p.y]);
                s.push_str(ops.moveto);
                s.push('\n');
                (last, start) = (p, p);
            }
            PathEl::LineTo(p) => {
                push_nums(s, &[p.x, p.y]);
                s.push_str(ops.lineto);
                s.push('\n');
                last = p;
            }
            PathEl::QuadTo(a, p) => {
                let c1 = last + (a - last) * (2.0 / 3.0);
                let c2 = p + (a - p) * (2.0 / 3.0);
                push_nums(s, &[c1.x, c1.y, c2.x, c2.y, p.x, p.y]);
                s.push_str(ops.curveto);
                s.push('\n');
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                push_nums(s, &[a.x, a.y, b.x, b.y, p.x, p.y]);
                s.push_str(ops.curveto);
                s.push('\n');
                last = p;
            }
            PathEl::ClosePath => {
                s.push_str(ops.closepath);
                s.push('\n');
                last = start;
            }
        }
    }
}

/// The winding rule a fill or clip uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// Operator names written for a fill or a clip (which also ends the path).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintOps {
    pub fill: &'static str,
    pub eofill: &'static str,
    pub clip: &'static str,
    pub eoclip: &'static str,
}

impl PaintOps {
    /// The PostScript Language Reference names. The clip operators also start a new path.
    pub const STANDARD: Self = Self { fill: "fill\n", eofill: "eofill\n", clip: "clip newpath\n", eoclip: "eoclip newpath\n" };
    /// Short names a bound prolog can define (`f`, `f*`, `W`, `W*`).
    pub const SHORT: Self = Self { fill: "f\n", eofill: "f*\n", clip: "W\n", eoclip: "W*\n" };
}

/// The fill operator for `rule`.
pub fn fill_op(rule: FillRule, ops: PaintOps) -> &'static str {
    match rule {
        FillRule::NonZero => ops.fill,
        FillRule::EvenOdd => ops.eofill,
    }
}

/// The clip operator for `rule` (it also ends the path).
pub fn clip_op(rule: FillRule, ops: PaintOps) -> &'static str {
    match rule {
        FillRule::NonZero => ops.clip,
        FillRule::EvenOdd => ops.eoclip,
    }
}

/// `data` compressed for `FlateDecode` (zlib).
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::with_capacity(data.len() / 2), flate2::Compression::default());
    // Writing to a Vec doesn't fail.
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

/// zlib `data` inflated, when it is valid and at most `max` bytes.
pub fn inflate(data: &[u8], max: u64) -> Option<Vec<u8>> {
    let mut out = vec![];
    flate2::read::ZlibDecoder::new(data).take(max.saturating_add(1)).read_to_end(&mut out).ok()?;
    (out.len() as u64 <= max).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::{
        FillRule, PaintOps, PathOps, bytes, clip_op, deflate, fill_op, inflate, matrix, num, push_num, push_num_places, push_nums, push_path, string,
        written,
    };
    use kurbo::{Affine, BezPath};

    #[test]
    fn huge_finite_numbers_are_valid_postscript() {
        for v in [1.5e305, -1.5e305, f64::MAX, f64::MIN] {
            let s = num(v);
            assert!(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.'), "{v}: got {s:?}");
        }
    }

    #[test]
    fn numbers_keep_the_decimals_asked_for() {
        let places = |v: f64, n: u32| {
            let mut s = String::new();
            push_num_places(&mut s, v, n);
            s
        };
        assert_eq!(num(0.123_456_78), "0.1235");
        assert_eq!(places(0.123_456_78, 7), "0.1234568");
        assert_eq!(places(f64::from(0.4f32), 7), "0.4");
        assert_eq!(places(2.5, 7), "2.5");
        assert_eq!(places(f64::NAN, 7), "0");
        assert_eq!(places(f64::INFINITY, 2), "0");
        assert_eq!(places(f64::NEG_INFINITY, 2), "0");
        assert_eq!(num(-2.0), "-2");
        assert_eq!(num(0.0), "0");
        assert_eq!(written(0.123_456_78), 0.1235);
        let mut s = String::new();
        push_num(&mut s, 3.0);
        assert_eq!(s, "3");
        let mut nums = String::new();
        push_nums(&mut nums, &[1.0, 2.5]);
        assert_eq!(nums, "1 2.5 ");
        assert_eq!(places(1.0, 20), "1");
        assert_eq!(places(1.23456789, 9), "1.23456789");
        assert_eq!(places(1e15, 0), "1000000000000000");
        assert_eq!(places(1e15 - 1.0, 0), "999999999999999");
    }

    #[test]
    fn strings_and_bytes_are_escaped() {
        assert_eq!(string("ok"), "(ok)");
        assert_eq!(string("a (b) \\c"), "(a \\(b\\) \\\\c)");
        assert_eq!(string("ä"), "(\\303\\244)");
        assert_eq!(bytes(b"ok"), "(ok)");
        assert_eq!(bytes(b"(\r\n\t)"), "(\\(\\r\\n\\t\\))");
        assert_eq!(bytes(&[0x01, 0xff]), "(\\001\\377)");
        assert_eq!(string("a\nb"), "(a\\012b)");
    }

    #[test]
    fn matrix_and_paths_use_the_operator_names_asked_for() {
        assert_eq!(matrix(Affine::new([1.0, 0.0, 0.0, 1.0, 10.0, 20.0])), "[1 0 0 1 10 20]");
        let mut path = BezPath::new();
        path.move_to((1.0, 2.0));
        path.line_to((3.0, 4.0));
        path.quad_to((5.0, 6.0), (7.0, 8.0));
        path.curve_to((9.0, 10.0), (11.0, 12.0), (13.0, 14.0));
        path.close_path();
        let mut short = String::new();
        push_path(&mut short, &path, PathOps::SHORT);
        assert_eq!(short, "1 2 m\n3 4 l\n4.3333 5.3333 5.6667 6.6667 7 8 c\n9 10 11 12 13 14 c\nh\n");
        let mut standard = String::new();
        push_path(&mut standard, &path, PathOps::STANDARD);
        assert!(
            standard.contains("moveto\n") && standard.contains("lineto\n") && standard.contains("curveto\n") && standard.ends_with("closepath\n")
        );
        let mut empty = String::new();
        push_path(&mut empty, &BezPath::new(), PathOps::STANDARD);
        assert!(empty.is_empty());
        assert_eq!(fill_op(FillRule::NonZero, PaintOps::SHORT), "f\n");
        assert_eq!(fill_op(FillRule::EvenOdd, PaintOps::SHORT), "f*\n");
        assert_eq!(clip_op(FillRule::NonZero, PaintOps::SHORT), "W\n");
        assert_eq!(clip_op(FillRule::EvenOdd, PaintOps::SHORT), "W*\n");
        assert_eq!(fill_op(FillRule::NonZero, PaintOps::STANDARD), "fill\n");
        assert_eq!(fill_op(FillRule::EvenOdd, PaintOps::STANDARD), "eofill\n");
        assert_eq!(clip_op(FillRule::NonZero, PaintOps::STANDARD), "clip newpath\n");
        assert_eq!(clip_op(FillRule::EvenOdd, PaintOps::STANDARD), "eoclip newpath\n");
    }

    #[test]
    fn flate_round_trips_and_honours_the_size_limit() {
        let data = b"PostScript FlateDecode payload \x00\xff".repeat(32);
        let compressed = deflate(&data);
        assert_ne!(compressed, data);
        assert_eq!(inflate(&compressed, data.len() as u64).as_deref(), Some(data.as_slice()));
        assert_eq!(inflate(&compressed, (data.len() as u64) - 1), None);
        assert_eq!(inflate(b"not zlib", 64), None);
        assert_eq!(inflate(&deflate(b""), 0).as_deref(), Some(&b""[..]));
        let hello = deflate(b"hello");
        assert_eq!(inflate(&hello, u64::MAX).as_deref(), Some(&b"hello"[..]));
    }
}
