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
    if push_num_scaled(s, v, places) {
        return;
    }
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

/// `10^places` for `places` up to 9, each exact in an `f64`.
const POW10: [f64; 10] = [1.0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9];

/// 2^52: below it every integer is an `f64`, and dividing one by `10^places` lands within half a
/// decimal step of the exact quotient, so the decimal digits are those of the integer.
const SCALED_LIMIT: f64 = 4_503_599_627_370_496.0;

/// Append `v` with at most `places` decimals by writing the digits of the rounded integer
/// `v * 10^places`, the same text [`push_num_places`] writes. Returns false and appends nothing
/// when `places` is above 9, `v` is not finite, the scaling overflows or the rounded integer is
/// 2^52 or more in magnitude.
fn push_num_scaled(s: &mut String, v: f64, places: u32) -> bool {
    let Some(&scale) = POW10.get(places as usize) else {
        return false;
    };
    let k = (v * scale).round();
    let magnitude = k.abs();
    if magnitude.is_nan() || magnitude >= SCALED_LIMIT {
        return false;
    }
    // `magnitude` is a whole number below 2^52, so the cast is exact.
    let mut m = magnitude as u64;
    let mut decimals = places as usize;
    while decimals > 0 && m.is_multiple_of(10) {
        m /= 10;
        decimals -= 1;
    }
    let unit = POW10_INT.get(decimals).copied().unwrap_or(1);
    let (whole, fraction) = (m / unit, m % unit);
    // A rounded negative zero has no sign: it is written `0`.
    if k < 0.0 {
        s.push('-');
    }
    push_digits(s, whole, POW10_INT.iter().take_while(|&&p| p <= whole).count().max(1));
    if decimals > 0 {
        s.push('.');
        push_digits(s, fraction, decimals);
    }
    true
}

/// `10^n` for `n` up to 16, enough for every digit of an integer below 2^52.
const POW10_INT: [u64; 17] = [
    1,
    10,
    100,
    1_000,
    10_000,
    100_000,
    1_000_000,
    10_000_000,
    100_000_000,
    1_000_000_000,
    10_000_000_000,
    100_000_000_000,
    1_000_000_000_000,
    10_000_000_000_000,
    100_000_000_000_000,
    1_000_000_000_000_000,
    10_000_000_000_000_000,
];

/// Append the last `width` decimal digits of `n`, leading zeros included.
fn push_digits(s: &mut String, n: u64, width: usize) {
    for &p in POW10_INT.iter().take(width).rev() {
        s.push(char::from(b'0' + ((n / p) % 10) as u8));
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
        FillRule, PaintOps, PathOps, bytes, clip_op, deflate, fill_op, inflate, matrix, num, push_num, push_num_places, push_num_scaled, push_nums,
        push_path, string, written,
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

    /// `push_num_places` without the integer path: every value goes through the float formatter.
    fn base_push_num_places(s: &mut String, v: f64, places: u32) {
        use std::fmt::Write;
        let places = places.min(9);
        let scale = 10f64.powi(places as i32);
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
        if s[start..].contains('.') {
            let keep = s[start..].trim_end_matches('0').trim_end_matches('.').len();
            s.truncate(start + keep);
        }
    }

    /// A deterministic xorshift sequence.
    fn next(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// Values that hit decimal ties, the 2^52 and 1e15 limits and their neighboring bit patterns,
    /// coordinates of every magnitude, and arbitrary bit patterns.
    fn number_samples() -> Vec<f64> {
        let two52 = 4_503_599_627_370_496.0f64;
        let mut values = vec![
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            -0.5,
            1.5,
            2.5,
            -2.5,
            0.1 + 0.2,
            f64::from(0.4f32),
            0.123_456_78,
            123_456.789_012_345,
            -0.000_04,
            -0.000_05,
            0.000_05,
            1e-10,
            -1e-10,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            5e-324,
            1e15,
            1e15 - 1.0,
            -1e15,
            1e15 + 0.5,
            two52,
            two52 - 1.0,
            two52 - 0.5,
            -two52,
            1e16,
            1e20,
            1.5e305,
            f64::MAX,
            f64::MIN,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut scale = 1.0;
        for _ in 0..=9 {
            for v in [two52 / scale, (two52 - 1.0) / scale, 1e15 / scale, 0.5 / scale, 1.5 / scale, 2.5 / scale] {
                values.extend([v, -v]);
            }
            scale *= 10.0;
        }
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..12_000 {
            let r = next(&mut state);
            values.push(f64::from_bits(r));
            let exponent = (r % 24) as i32 - 8;
            let mantissa = (next(&mut state) % 2_000_000_001) as f64 / 1e9 - 1.0;
            values.push(mantissa * 10f64.powi(exponent));
            let places = (r % 10) as i32;
            let tie = ((next(&mut state) % 20_000_000_001) as f64 - 1e10 + 0.5) / 10f64.powi(places);
            values.push(tie);
            values.push((next(&mut state) % 100_000_000) as f64 / 1e4 - 5_000.0);
        }
        let neighbors: Vec<f64> = values
            .iter()
            .filter(|v| v.is_finite() && **v != 0.0)
            .flat_map(|v| [f64::from_bits(v.to_bits() - 1), f64::from_bits(v.to_bits() + 1)])
            .collect();
        values.extend(neighbors);
        values
    }

    #[test]
    fn numbers_match_base_formatter() {
        let mut integer_path = 0usize;
        for places in 0..=12 {
            for v in number_samples() {
                let mut expected = String::from("x ");
                base_push_num_places(&mut expected, v, places);
                let mut got = String::from("x ");
                push_num_places(&mut got, v, places);
                assert_eq!(got, expected, "{v:e} ({:#x}) at {places} places", v.to_bits());
                if push_num_scaled(&mut String::new(), v, places.min(9)) {
                    integer_path += 1;
                }
            }
        }
        // The comparison covers the integer path, not only the fallback.
        assert!(integer_path > 200_000, "{integer_path} values took the integer path");
    }

    #[test]
    fn coordinates_are_written_through_integers() {
        for (v, places) in [(0.0, 4), (-0.0, 4), (1.5, 4), (-2.25, 4), (4.3333, 4), (612.0, 0), (0.123_456_78, 7), (-0.000_05, 4), (1.23456789, 9)] {
            let mut s = String::from("x ");
            assert!(push_num_scaled(&mut s, v, places), "{v} at {places} places");
            let mut expected = String::from("x ");
            base_push_num_places(&mut expected, v, places);
            assert_eq!(s, expected);
        }
        let two52 = 4_503_599_627_370_496.0f64;
        for (v, places) in [
            (f64::NAN, 4),
            (f64::INFINITY, 4),
            (f64::NEG_INFINITY, 0),
            (f64::MAX, 4),
            (f64::MIN, 0),
            (1e305, 9),
            (two52, 0),
            (-two52, 0),
            (two52 / 1e9, 9),
            (1e20, 0),
            (1.5, 10),
        ] {
            let mut s = String::from("x ");
            assert!(!push_num_scaled(&mut s, v, places), "{v} at {places} places");
            assert_eq!(s, "x ");
        }
    }

    #[test]
    fn number_boundaries_keep_base_text() {
        let places = |v: f64, n: u32| {
            let mut s = String::new();
            push_num_places(&mut s, v, n);
            s
        };
        let two52 = 4_503_599_627_370_496.0f64;
        assert_eq!(places(1e15, 0), "1000000000000000");
        assert_eq!(places(-1e15, 0), "-1000000000000000");
        assert_eq!(places(two52 - 1.0, 0), "4503599627370495");
        assert_eq!(places(two52, 0), "4503599627370496");
        assert_eq!(places(1e20, 0), "100000000000000000000");
        assert_eq!(places(450_359_962_737_049.5, 1), "450359962737049.5");
        assert_eq!(places(-0.0, 4), "0");
        assert_eq!(places(-0.000_04, 4), "0");
        assert_eq!(places(-0.000_05, 4), "-0.0001");
        assert_eq!(places(0.5, 0), "1");
        assert_eq!(places(-1.5, 0), "-2");
        assert_eq!(places(0.000_000_001, 9), "0.000000001");
        assert_eq!(places(1.25, 20), "1.25");
        assert_eq!(places(10.5, 4), "10.5");
        assert_eq!(places(100.0, 4), "100");
        let mut s = String::from("x");
        push_nums(&mut s, &[]);
        assert_eq!(s, "x");
        let mut scale = 1.0;
        for (n, value) in super::POW10.iter().enumerate() {
            assert_eq!(*value, scale, "10^{n}");
            assert_eq!(*value, 10f64.powi(n as i32), "10^{n}");
            scale *= 10.0;
        }
    }

    /// Times the base formatter against `push_num_places` on path coordinates. Run with
    /// `cargo test --release -p craft-ps --features write number_writing_timing -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing probe"]
    fn number_writing_timing() {
        use std::time::Instant;
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let values: Vec<f64> = (0..1_200_000).map(|_| (next(&mut state) % 12_000_000_000) as f64 / 1e7 - 200.0).collect();
        let mut base_ms = vec![];
        let mut new_ms = vec![];
        for _ in 0..7 {
            let start = Instant::now();
            let mut base = String::with_capacity(values.len() * 10);
            for v in &values {
                base_push_num_places(&mut base, *v, 4);
                base.push(' ');
            }
            base_ms.push(start.elapsed().as_secs_f64() * 1e3);
            let start = Instant::now();
            let mut new = String::with_capacity(values.len() * 10);
            push_nums(&mut new, &values);
            new_ms.push(start.elapsed().as_secs_f64() * 1e3);
            assert_eq!(new, base);
        }
        println!("{} numbers at 4 places", values.len());
        println!("base formatter ms: {base_ms:.2?}");
        println!("integer path ms: {new_ms:.2?}");
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
