//! XFL's compact shape-edge attribute grammar.
//!
//! An edge's `edges` attribute uses twips (1/20 pixel): `!x y` moves, `|x y` and `/x y` draw a
//! line, `[cx cy x y` and `]cx cy x y` draw a quadratic curve, and `Sn` marks selection state.
//! Coordinates may be decimal or signed 24.8 fixed-point hexadecimal (`#A.8` is 10.5).
//!
//! This module returns neutral points and segments so consumers can map them to their own
//! geometry model. Like Animate files in the wild, a damaged suffix ends parsing while keeping
//! the valid prefix. The segment cap bounds output allocation.

/// Most segments parsed from one edge attribute.
pub const MAX_EDGE_SEGMENTS: usize = 2_000_000;

/// A point in XFL's edge geometry, expressed in pixels after twips conversion.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// One line (`control` is absent) or quadratic curve segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub from: Point,
    pub control: Option<Point>,
    pub to: Point,
}

const TWIPS_PER_PIXEL: f32 = 20.0;

fn number(input: &[u8], at: &mut usize) -> Option<f32> {
    while input.get(*at).is_some_and(|b| b.is_ascii_whitespace()) {
        *at += 1;
    }
    let start = *at;
    if input.get(*at) == Some(&b'#') {
        *at += 1;
        let integer_start = *at;
        while input.get(*at).is_some_and(u8::is_ascii_hexdigit) {
            *at += 1;
        }
        let integer = std::str::from_utf8(input.get(integer_start..*at)?).ok()?;
        let mut fraction = 0_u32;
        if input.get(*at) == Some(&b'.') {
            *at += 1;
            let fraction_start = *at;
            while input.get(*at).is_some_and(u8::is_ascii_hexdigit) && *at - fraction_start < 2 {
                *at += 1;
            }
            let digits = std::str::from_utf8(input.get(fraction_start..*at)?).ok()?;
            // A single hex digit is the high nibble of the 8-bit fractional part.
            fraction = u32::from_str_radix(digits, 16).ok()? << if digits.len() == 1 { 4 } else { 0 };
        }
        let whole = u32::from_str_radix(if integer.is_empty() { "0" } else { integer }, 16).ok()?;
        let fixed = whole.wrapping_shl(8) | fraction;
        return Some((fixed as i32) as f32 / 256.0);
    }
    if matches!(input.get(*at), Some(b'-' | b'+')) {
        *at += 1;
    }
    while input.get(*at).is_some_and(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E')) {
        *at += 1;
    }
    std::str::from_utf8(input.get(start..*at)?).ok()?.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn point(input: &[u8], at: &mut usize) -> Option<Point> {
    Some(Point { x: number(input, at)? / TWIPS_PER_PIXEL, y: number(input, at)? / TWIPS_PER_PIXEL })
}

/// Parses an XFL `<Edge edges="…">` attribute into pixel-coordinate line and quadratic segments.
///
/// Stops at the first unknown or malformed command, preserving the valid prefix. The current
/// point starts at the origin. Move and selection commands do not produce segments.
pub fn parse_edge_segments(source: &str) -> Vec<Segment> {
    let input = source.as_bytes();
    let mut at = 0;
    let mut current = Point::default();
    let mut segments = Vec::new();
    while let Some(&command) = input.get(at) {
        at += 1;
        match command {
            b'!' => match point(input, &mut at) {
                Some(p) => current = p,
                None => break,
            },
            b'|' | b'/' => match point(input, &mut at) {
                Some(to) => {
                    segments.push(Segment { from: current, control: None, to });
                    current = to;
                }
                None => break,
            },
            b'[' | b']' => match (point(input, &mut at), point(input, &mut at)) {
                (Some(control), Some(to)) => {
                    segments.push(Segment { from: current, control: Some(control), to });
                    current = to;
                }
                _ => break,
            },
            b'S' => {
                while input.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
            }
            b if b.is_ascii_whitespace() => {}
            _ => break,
        }
        if segments.len() >= MAX_EDGE_SEGMENTS {
            break;
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_line_quadratic_and_twips() {
        let segments = parse_edge_segments("!0 0|200 0[400 0 400 200S2/0 200");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].to, Point { x: 10.0, y: 0.0 });
        assert_eq!(segments[1].control, Some(Point { x: 20.0, y: 0.0 }));
        assert_eq!(segments[2], Segment { from: Point { x: 20.0, y: 10.0 }, control: None, to: Point { x: 0.0, y: 10.0 } });
    }

    #[test]
    fn parses_signed_24_8_fixed_hex_coordinates() {
        let segments = parse_edge_segments("!#A.8 -20|#FFFFFF.80 0");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].from, Point { x: 0.525, y: -1.0 });
        assert_eq!(segments[0].to, Point { x: -0.025, y: 0.0 });
    }

    #[test]
    fn malformed_suffix_keeps_only_valid_prefix() {
        let segments = parse_edge_segments("!0 0|200 0[400 0 nope 200");
        assert_eq!(segments, [Segment { from: Point::default(), control: None, to: Point { x: 10.0, y: 0.0 } }]);
        assert!(parse_edge_segments("!1 2|x").is_empty());
        assert!(parse_edge_segments("!NaN 2|5 5").is_empty());
    }

    #[test]
    fn oversized_coordinate_numbers_and_segment_lists_are_bounded() {
        assert!(parse_edge_segments("!999999999999999999999999999999999999999999999999 0|1 1").is_empty());
        let source = "|0 0".repeat(MAX_EDGE_SEGMENTS + 1);
        let segments = parse_edge_segments(&source);
        assert_eq!(segments.len(), MAX_EDGE_SEGMENTS);
    }
}
