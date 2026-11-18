//! HATCH: the boundary loops (polylines or edges), the pattern definition, the seed points and
//! the gradient, as the file holds them. Boundary points are 2D, in the object coordinate system
//! of the hatch at its elevation.

use super::{Color, Groups, parse_handle};
use crate::Tag;

/// One edge of a boundary loop made of edges.
#[derive(Clone, Debug, PartialEq)]
pub enum HatchEdge {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    /// Angles in degrees; a clockwise arc (`ccw` false) has them measured clockwise.
    Arc {
        center: [f64; 2],
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        ccw: bool,
    },
    /// `major` is the end of the major axis from the centre, `ratio` the minor axis's length
    /// over the major's; angles in degrees, as for an arc.
    Ellipse {
        center: [f64; 2],
        major: [f64; 2],
        ratio: f64,
        start_angle: f64,
        end_angle: f64,
        ccw: bool,
    },
    Spline {
        degree: i64,
        rational: bool,
        periodic: bool,
        knots: Vec<f64>,
        control: Vec<[f64; 2]>,
        /// One per control point when rational; else empty.
        weights: Vec<f64>,
        fit: Vec<[f64; 2]>,
        start_tangent: Option<[f64; 2]>,
        end_tangent: Option<[f64; 2]>,
    },
}

/// The path of a boundary loop.
#[derive(Clone, Debug, PartialEq)]
pub enum HatchPath {
    /// Vertices as x, y and bulge.
    Polyline {
        closed: bool,
        vertices: Vec<[f64; 3]>,
    },
    Edges(Vec<HatchEdge>),
}

/// A boundary loop.
#[derive(Clone, Debug, PartialEq)]
pub struct HatchLoop {
    /// Group 92: 1 external, 2 a polyline, 4 derived, 8 a text box, 16 outermost.
    pub flags: i64,
    pub path: HatchPath,
    /// Groups 330 after the loop's 97: the entities an associative hatch follows.
    pub sources: Vec<u64>,
}

/// One line family of a pattern definition, as drawn: scaled and rotated with the hatch.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PatternLine {
    /// Group 53, in degrees.
    pub angle: f64,
    /// Groups 43/44: a point the first line passes through.
    pub base: [f64; 2],
    /// Groups 45/46: the step from one line to the next, as a vector.
    pub offset: [f64; 2],
    /// Groups 49: dash (positive), gap (negative) and dot (0) lengths; empty for a solid line.
    pub dashes: Vec<f64>,
}

/// A gradient fill (groups 450–470).
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    /// Group 470 (`LINEAR`, `CYLINDER`, `SPHERICAL`...).
    pub name: String,
    /// Group 460, in radians.
    pub angle: f64,
    /// Group 461: 0 centred, 1 shifted up and left.
    pub shift: f64,
    /// Group 452 is 1: one colour, shaded by `tint` (group 462).
    pub single: bool,
    pub tint: f64,
    /// The colour of each 463 record: its true colour (421), else its index (63).
    pub colors: Vec<Color>,
}

/// A HATCH.
#[derive(Clone, Debug, PartialEq)]
pub struct Hatch {
    /// Group 2, as written (`SOLID` when absent).
    pub pattern: String,
    /// Group 70 is 1.
    pub solid: bool,
    /// Group 71 is not 0.
    pub associative: bool,
    /// Group 75: 0 normal (odd parity), 1 outer, 2 ignore islands.
    pub style: i64,
    /// Group 76: 0 user-defined, 1 predefined, 2 custom.
    pub pattern_type: i64,
    /// Group 52, in degrees.
    pub angle: f64,
    /// Group 41 (1 when absent).
    pub scale: f64,
    /// Group 77 is not 0: a user-defined pattern is doubled (crossed).
    pub double: bool,
    /// The z of the elevation point (the first group 30).
    pub elevation: f64,
    pub loops: Vec<HatchLoop>,
    pub pattern_lines: Vec<PatternLine>,
    /// Groups 10/20 after the 98 count.
    pub seeds: Vec<[f64; 2]>,
    pub gradient: Option<Gradient>,
}

/// A cursor over a record's tags, for the boundary data, which is read in order.
struct Cur<'a> {
    tags: &'a [Tag],
    at: usize,
}

impl<'a> Cur<'a> {
    fn peek(&self) -> Option<&'a Tag> {
        self.tags.get(self.at)
    }
    /// The next tag when its code is `code`, consumed.
    fn take(&mut self, code: i32) -> Option<&'a Tag> {
        let t = self.peek().filter(|t| t.code == code)?;
        self.at += 1;
        Some(t)
    }
    fn f(&mut self, code: i32) -> f64 {
        self.take(code).map_or(0.0, Tag::f64)
    }
    fn xy(&mut self, code: i32) -> [f64; 2] {
        [self.f(code), self.f(code + 10)]
    }
    /// Move to the next `code`; false when there is none.
    fn seek(&mut self, code: i32) -> bool {
        while self.peek().is_some_and(|t| t.code != code) {
            self.at += 1;
        }
        self.peek().is_some()
    }
}

/// A declared count: never trusted for an allocation, only as the most items to read.
fn count(t: &Tag) -> usize {
    usize::try_from(t.i64()).unwrap_or(0)
}

fn polyline_path(c: &mut Cur<'_>) -> Option<HatchPath> {
    // 72 has bulges, 73 closed, 93 the vertex count.
    let mut closed = false;
    while let Some(t) = c.peek().filter(|t| t.code != 93) {
        if t.code == 73 {
            closed = t.i64() != 0;
        } else if t.code != 72 {
            return None;
        }
        c.at += 1;
    }
    let n = count(c.take(93)?);
    let mut vertices = Vec::new();
    while vertices.len() < n && c.peek().is_some_and(|t| t.code == 10) {
        let [x, y] = c.xy(10);
        vertices.push([x, y, c.f(42)]);
    }
    Some(HatchPath::Polyline { closed, vertices })
}

fn spline_edge(c: &mut Cur<'_>) -> HatchEdge {
    let (mut degree, mut rational, mut periodic, mut knot_count, mut control_count) = (3, false, false, 0, 0);
    while let Some(t) = c.peek().filter(|t| matches!(t.code, 94 | 73 | 74 | 95 | 96)) {
        match t.code {
            94 => degree = t.i64(),
            73 => rational = t.i64() != 0,
            74 => periodic = t.i64() != 0,
            95 => knot_count = count(t),
            _ => control_count = count(t),
        }
        c.at += 1;
    }
    let mut knots = Vec::new();
    while knots.len() < knot_count && c.peek().is_some_and(|t| t.code == 40) {
        knots.push(c.f(40));
    }
    let (mut control, mut weights) = (Vec::new(), Vec::new());
    while control.len() < control_count && c.peek().is_some_and(|t| t.code == 10) {
        control.push(c.xy(10));
        weights.extend(c.take(42).map(Tag::f64));
    }
    if weights.len() != control.len() {
        weights.clear();
    }
    // A 97 here is the edge's fit point count (R2010 and later) when fit points follow it, or
    // when it is 0 and the loop's own 97 comes next; otherwise it is the loop's source count.
    let mut fit = Vec::new();
    let next = c.tags.get(c.at + 1).map(|t| t.code);
    let fit_count = c.peek().filter(|t| t.code == 97 && (next == Some(11) || (t.i64() == 0 && next == Some(97))));
    if let Some(n) = fit_count.map(count) {
        c.at += 1;
        while fit.len() < n && c.peek().is_some_and(|t| t.code == 11) {
            fit.push(c.xy(11));
        }
    }
    let tangent = |c: &mut Cur<'_>, code: i32| c.peek().is_some_and(|t| t.code == code).then(|| c.xy(code));
    let start_tangent = tangent(c, 12);
    let end_tangent = tangent(c, 13);
    HatchEdge::Spline { degree, rational, periodic, knots, control, weights, fit, start_tangent, end_tangent }
}

fn edge_path(c: &mut Cur<'_>) -> Option<HatchPath> {
    if !c.seek(93) {
        return None;
    }
    let n = count(c.take(93)?);
    let mut edges = Vec::new();
    while edges.len() < n {
        // Groups this reader does not know may sit between edges; the loop's own end may not.
        while c.peek().is_some_and(|t| !matches!(t.code, 72 | 97 | 92 | 75)) {
            c.at += 1;
        }
        if c.peek().is_none_or(|t| t.code != 72) {
            break;
        }
        let kind = c.take(72).map_or(0, Tag::i64);
        // 73 is counter-clockwise unless it says 0.
        let ccw = |c: &mut Cur<'_>| c.take(73).is_none_or(|t| t.i64() != 0);
        edges.push(match kind {
            1 => HatchEdge::Line { start: c.xy(10), end: c.xy(11) },
            2 => HatchEdge::Arc { center: c.xy(10), radius: c.f(40), start_angle: c.f(50), end_angle: c.f(51), ccw: ccw(c) },
            3 => HatchEdge::Ellipse { center: c.xy(10), major: c.xy(11), ratio: c.f(40), start_angle: c.f(50), end_angle: c.f(51), ccw: ccw(c) },
            4 => spline_edge(c),
            // An edge type the format does not have: its groups are passed over above.
            _ => continue,
        });
    }
    Some(HatchPath::Edges(edges))
}

fn pattern_lines(tags: &[Tag]) -> Vec<PatternLine> {
    let Some(at) = tags.iter().position(|t| t.code == 78) else { return Vec::new() };
    let mut lines: Vec<PatternLine> = Vec::new();
    for t in tags.iter().skip(at + 1) {
        if t.code == 53 {
            lines.push(PatternLine { angle: t.f64(), ..PatternLine::default() });
            continue;
        }
        let Some(l) = lines.last_mut() else { break };
        match t.code {
            43 => l.base[0] = t.f64(),
            44 => l.base[1] = t.f64(),
            45 => l.offset[0] = t.f64(),
            46 => l.offset[1] = t.f64(),
            79 => {}
            49 => l.dashes.push(t.f64()),
            _ => break,
        }
    }
    lines
}

/// Groups 450–470, when 450 says gradient. Each 463 starts a colour record whose 63 (index) and
/// 421 (true colour) follow.
fn gradient(tags: &[Tag]) -> Option<Gradient> {
    let start = tags.iter().position(|t| t.code == 450)?;
    if tags.get(start)?.i64() != 1 {
        return None;
    }
    let mut g = Gradient { name: String::new(), angle: 0.0, shift: 0.0, single: false, tint: 0.0, colors: Vec::new() };
    for t in tags.iter().skip(start + 1).take_while(|t| t.code != 1001) {
        match t.code {
            452 => g.single = t.i64() == 1,
            460 => g.angle = t.f64(),
            461 => g.shift = t.f64(),
            462 => g.tint = t.f64(),
            463 => g.colors.push(Color::ByLayer),
            63 => {
                // A true colour already read for this record wins over its index.
                if let Some(c) = g.colors.last_mut().filter(|c| !matches!(c, Color::Rgb(_))) {
                    *c = Color::from_index(t.i64());
                }
            }
            421 => {
                if let Some(c) = g.colors.last_mut() {
                    *c = Color::from_true(t.i64());
                }
            }
            470 => g.name = t.str(),
            _ => {}
        }
    }
    Some(g)
}

impl Hatch {
    /// The typed form of a HATCH record; `None` when it has no boundary data (no group 91) or
    /// the data ends inside a loop.
    pub fn read(tags: &[Tag]) -> Option<Hatch> {
        let g = Groups(tags);
        let n_loops = count(tags.iter().find(|t| t.code == 91)?);
        let mut c = Cur { tags, at: tags.iter().position(|t| t.code == 91)? + 1 };
        let mut loops = Vec::new();
        while loops.len() < n_loops {
            if !c.seek(92) {
                return None;
            }
            let flags = c.take(92)?.i64();
            let path = if flags & 2 != 0 { polyline_path(&mut c)? } else { edge_path(&mut c)? };
            // 97 counts the source entities (330) that follow.
            let mut sources = Vec::new();
            if let Some(n) = c.take(97).map(count) {
                while sources.len() < n
                    && let Some(t) = c.take(330)
                {
                    sources.extend(parse_handle(&t.str()).filter(|h| *h != 0));
                }
            }
            loops.push(HatchLoop { flags, path, sources });
        }
        let mut seeds = Vec::new();
        if c.seek(98) {
            let n = c.take(98).map_or(0, count);
            while seeds.len() < n && c.peek().is_some_and(|t| t.code == 10) {
                seeds.push(c.xy(10));
            }
        }
        let pattern = g.name(2).unwrap_or_else(|| "SOLID".into());
        Some(Hatch {
            solid: g.i_or(70, 0) == 1,
            associative: g.i_or(71, 0) != 0,
            style: g.i_or(75, 0),
            pattern_type: g.i_or(76, 1),
            angle: g.f_or(52, 0.0),
            scale: g.f_or(41, 1.0),
            double: g.i_or(77, 0) != 0,
            elevation: g.f_or(30, 0.0),
            loops,
            pattern_lines: pattern_lines(tags),
            seeds,
            gradient: gradient(tags),
            pattern,
        })
    }
}
