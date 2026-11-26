//! XFL's compact shape-edge attribute grammar.
//!
//! An edge's `edges` attribute uses twips (1/20 pixel): `!x y` moves, `|x y` and `/x y` draw a
//! line, `[cx cy x y` and `]cx cy x y` draw a quadratic curve, and `Sn` marks selection state.
//! Coordinates may be decimal or signed 24.8 fixed-point hexadecimal (`#A.8` is 10.5).
//!
//! This module returns neutral points and segments so consumers can map them to their own
//! geometry model. Each `!` starts a run that depends on nothing before it, so an unknown or
//! malformed command ends only its own run: the run's valid prefix is kept and parsing resumes
//! at the next `!`. [`parse_edges`](crate::edges::parse_edges) reports how many runs were cut
//! short. The segment cap bounds output allocation.

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

/// Segments parsed from one edge attribute, with a count of the runs that were not read whole.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedEdges {
    pub segments: Vec<Segment>,
    /// Runs that ended early at an unknown or malformed command, or that the segment cap cut off
    /// or left unread. A run's segments before the point where it ended are still in `segments`.
    pub cut_runs: usize,
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
    while let Some(&b) = input.get(*at) {
        if b.is_ascii_digit() || b == b'.' {
            *at += 1;
        } else if matches!(b, b'e' | b'E') {
            *at += 1;
            if matches!(input.get(*at), Some(b'-' | b'+')) {
                *at += 1;
            }
        } else {
            break;
        }
    }
    std::str::from_utf8(input.get(start..*at)?).ok()?.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn point(input: &[u8], at: &mut usize) -> Option<Point> {
    Some(Point { x: number(input, at)? / TWIPS_PER_PIXEL, y: number(input, at)? / TWIPS_PER_PIXEL })
}

/// Parses an XFL `<Edge edges="…">` attribute into pixel-coordinate line and quadratic segments.
///
/// An unknown or malformed command ends its run, keeping the run's valid prefix, and parsing
/// resumes at the next `!`; use [`parse_edges`] to learn how many runs were cut short. The current
/// point starts at the origin. Move and selection commands do not produce segments.
pub fn parse_edge_segments(source: &str) -> Vec<Segment> {
    parse_edges(source).segments
}

/// Parses an XFL `<Edge edges="…">` attribute like [`parse_edge_segments`], and also counts the
/// runs that were cut short, so a consumer can report that part of the edge is missing.
pub fn parse_edges(source: &str) -> ParsedEdges {
    let input = source.as_bytes();
    let mut at = 0;
    let mut current = Point::default();
    let mut parsed = ParsedEdges::default();
    while let Some(&command) = input.get(at) {
        at += 1;
        let read = match command {
            b'!' => point(input, &mut at).map(|p| current = p),
            b'|' | b'/' => point(input, &mut at).map(|to| {
                parsed.segments.push(Segment { from: current, control: None, to });
                current = to;
            }),
            b'[' | b']' => point(input, &mut at).zip(point(input, &mut at)).map(|(control, to)| {
                parsed.segments.push(Segment { from: current, control: Some(control), to });
                current = to;
            }),
            b'S' => {
                while input.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
                Some(())
            }
            b if b.is_ascii_whitespace() => Some(()),
            _ => None,
        };
        if read.is_none() {
            parsed.cut_runs += 1;
            at = next_run(input, at);
        }
        if parsed.segments.len() >= MAX_EDGE_SEGMENTS {
            parsed.cut_runs += unread_runs(input.get(at..).unwrap_or_default());
            break;
        }
    }
    parsed
}

/// The index of the next run's `!` at or after `at`, or the end of the input.
fn next_run(input: &[u8], at: usize) -> usize {
    input.get(at..).and_then(|rest| rest.iter().position(|&b| b == b'!')).map_or(input.len(), |offset| at + offset)
}

/// Runs in `rest` that parsing will not reach: the current run if anything but whitespace is left
/// of it, and every later run.
fn unread_runs(rest: &[u8]) -> usize {
    let current_run = rest.iter().take_while(|&&b| b != b'!').any(|b| !b.is_ascii_whitespace());
    usize::from(current_run) + rest.iter().filter(|&&b| b == b'!').count()
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
    fn unreadable_command_ends_only_its_run() {
        let line = |from: (f32, f32), to: (f32, f32)| Segment { from: Point { x: from.0, y: from.1 }, control: None, to: Point { x: to.0, y: to.1 } };
        let parsed = parse_edges("!0 0|200 0?7|300 0!400 0|600 0");
        assert_eq!(parsed.segments, [line((0.0, 0.0), (10.0, 0.0)), line((20.0, 0.0), (30.0, 0.0))]);
        assert_eq!(parsed.cut_runs, 1);

        let parsed = parse_edges("!x 0|200 0!0 20[1 2 nope|5 5!0 0|0 200");
        assert_eq!(parsed.segments, [line((0.0, 0.0), (0.0, 10.0))]);
        assert_eq!(parsed.cut_runs, 2);
        assert_eq!(parse_edges("!0 0|200 0S2 !0 0|0 200").cut_runs, 0);
    }

    #[test]
    fn decimal_coordinates_take_a_signed_exponent() {
        let segments = parse_edge_segments("!0 0|2e+2 400E-1");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].to, Point { x: 10.0, y: 2.0 });
    }

    #[test]
    fn oversized_coordinate_numbers_and_segment_lists_are_bounded() {
        assert!(parse_edge_segments("!999999999999999999999999999999999999999999999999 0|1 1").is_empty());
        let source = "|0 0".repeat(MAX_EDGE_SEGMENTS + 1);
        let parsed = parse_edges(&source);
        assert_eq!(parsed.segments.len(), MAX_EDGE_SEGMENTS);
        assert_eq!(parsed.cut_runs, 1);
    }
}
