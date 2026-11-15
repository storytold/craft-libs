//! The shape pen program: vector length/direction bytes and special codes 0–14, run into
//! polylines. Bounded everywhere: subshape depth, location stack, steps, points, coordinates.
//!
//! - A byte with a non-zero high nibble is a vector: length (high nibble) in one of 16
//!   directions (low nibble, counter-clockwise from east in 22.5° steps; the off-axis ones
//!   are half-unit or unit steps on the minor axis).
//! - 0 end, 1 pen down, 2 pen up, 3/4 divide/multiply vector lengths by the next byte,
//!   5/6 push/pop the location, 7 subshape (one byte; two bytes high first in a Unicode font;
//!   `0`, a two-byte number, base point, width and height in an extended big font),
//!   8 one X-Y displacement, 9 displacements up to (0,0), 10 octant arc (radius,
//!   ±0SC: sign = clockwise, S start octant, C octants, 0 = 8), 11 fractional arc (start
//!   offset, end offset, high and low radius, ±0SC; offsets in 1/256 of 45° from the octant
//!   boundaries), 12 bulge arc (dx, dy, bulge: ±127 = semicircle), 13 bulge arcs up to (0,0),
//!   14 the next command is for vertical text only (dual-orientation fonts).
//! - Displacements and bulges are two's complement bytes; the arc direction byte keeps its
//!   sign in the high bit.

use std::f64::consts::{FRAC_PI_4, TAU};

use super::ShxKind;
use std::ops::{Add, Mul, Neg, Sub};

/// A point in SHX font units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
    fn len(self) -> f64 {
        self.x.hypot(self.y)
    }
    fn len2(self) -> f64 {
        self.x * self.x + self.y * self.y
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
impl Neg for Point {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

/// Font records and metrics the bounded shape interpreter needs.
pub trait Font {
    fn kind(&self) -> ShxKind;
    fn dual(&self) -> bool;
    fn above(&self) -> f64;
    fn char_width(&self) -> f64;
    fn program(&self, number: u16) -> Option<&[u8]>;
}
/// Pen-down strokes and advance in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glyph {
    pub strokes: Vec<Vec<Point>>,
    pub advance: Point,
}

/// Subshape nesting (a cycle ends here).
const MAX_DEPTH: usize = 16;
/// Location stack (the format promises four).
const MAX_STACK: usize = 64;
/// Bytes run per glyph, subshapes included.
const MAX_STEPS: usize = 200_000;
/// Points per glyph.
const MAX_POINTS: usize = 50_000;
/// Coordinates in font units.
const MAX_COORD: f64 = 1.0e7;
/// Arc segments per full turn.
const ARC_STEPS: f64 = 64.0;

/// Why a program stops early: the glyph is then not drawn.
#[derive(Debug)]
pub struct Broken;

type Res = Result<(), Broken>;

struct Pen<'a> {
    font: &'a dyn Font,
    vertical: bool,
    pos: Point,
    down: bool,
    /// Vector length factor (codes 3 and 4).
    scale: f64,
    /// Per-axis factor of an extended big font subshape.
    axes: Point,
    stack: Vec<Point>,
    strokes: Vec<Vec<Point>>,
    active: Vec<Point>,
    steps: usize,
    points: usize,
}

pub fn outline(font: &dyn Font, number: u16, vertical: bool) -> Result<Glyph, Broken> {
    let mut pen = Pen {
        font,
        vertical,
        pos: Point::ZERO,
        down: true,
        scale: 1.0,
        axes: Point::new(1.0, 1.0),
        stack: Vec::new(),
        strokes: Vec::new(),
        active: Vec::new(),
        steps: 0,
        points: 0,
    };
    pen.run(number, 0)?;
    pen.flush();
    Ok(Glyph { strokes: pen.strokes, advance: pen.pos })
}

/// The 16 vector directions (unit length on the major axis).
fn direction(d: u8) -> Point {
    let (x, y) = match d & 15 {
        0 => (1.0, 0.0),
        1 => (1.0, 0.5),
        2 => (1.0, 1.0),
        3 => (0.5, 1.0),
        4 => (0.0, 1.0),
        5 => (-0.5, 1.0),
        6 => (-1.0, 1.0),
        7 => (-1.0, 0.5),
        8 => (-1.0, 0.0),
        9 => (-1.0, -0.5),
        10 => (-1.0, -1.0),
        11 => (-0.5, -1.0),
        12 => (0.0, -1.0),
        13 => (0.5, -1.0),
        14 => (1.0, -1.0),
        _ => (1.0, -0.5),
    };
    Point::new(x, y)
}

/// A two's complement operand byte.
fn signed(b: u8) -> f64 {
    f64::from(i8::from_le_bytes([b]))
}

/// A program being read: its bytes and position.
struct Cur<'p> {
    b: &'p [u8],
    pos: usize,
}

impl Cur<'_> {
    fn byte(&mut self) -> Result<u8, Broken> {
        let v = self.b.get(self.pos).copied().ok_or(Broken)?;
        self.pos += 1;
        Ok(v)
    }
}

impl Pen<'_> {
    fn step(&mut self) -> Res {
        self.steps += 1;
        if self.steps > MAX_STEPS { Err(Broken) } else { Ok(()) }
    }

    fn flush(&mut self) {
        if self.active.len() >= 2 {
            self.strokes.push(std::mem::take(&mut self.active));
        } else {
            self.active.clear();
        }
    }

    /// Move the pen by `d` font units (already scaled), drawing when the pen is down.
    fn to(&mut self, target: Point) -> Res {
        if !target.is_finite() || target.x.abs().max(target.y.abs()) > MAX_COORD {
            return Err(Broken);
        }
        if self.down {
            if self.active.is_empty() {
                self.active.push(self.pos);
                self.points += 1;
            }
            self.active.push(target);
            self.points += 1;
            if self.points > MAX_POINTS {
                return Err(Broken);
            }
        } else {
            self.flush();
        }
        self.pos = target;
        Ok(())
    }

    /// A displacement in vector units (times the length factor and the subshape axes).
    fn by(&mut self, d: Point) -> Res {
        let k = d * self.scale;
        self.to(self.pos + Point::new(k.x * self.axes.x, k.y * self.axes.y))
    }

    /// An arc of `radius` vector units from the pen, starting at angle `start` on its circle
    /// and turning `sweep` radians (positive counter-clockwise).
    fn arc(&mut self, radius: f64, start: f64, sweep: f64) -> Res {
        let r = radius * self.scale;
        if !(r.is_finite() && r > 0.0 && sweep.is_finite()) {
            return Ok(());
        }
        let n = if self.down { (sweep.abs() / TAU * ARC_STEPS).ceil().clamp(1.0, ARC_STEPS) as usize } else { 1 };
        let from = self.pos;
        let s0 = Point::new(start.cos(), start.sin());
        for i in 1..=n {
            let a = start + sweep * i as f64 / n as f64;
            let d = (Point::new(a.cos(), a.sin()) - s0) * r;
            self.to(from + Point::new(d.x * self.axes.x, d.y * self.axes.y))?;
        }
        Ok(())
    }

    /// An arc to the displacement `d` with `bulge` (−127..127, ±127 a semicircle).
    fn bulge(&mut self, d: Point, bulge: f64) -> Res {
        let b = bulge.clamp(-127.0, 127.0) / 127.0;
        if b == 0.0 || !self.down || d.len2() == 0.0 {
            return self.by(d);
        }
        // Centre from the chord midpoint along its left normal; sweep 4·atan(b).
        let centre = d * 0.5 + Point::new(-d.y, d.x) * ((1.0 - b * b) / (4.0 * b));
        let rel = -centre;
        self.arc(rel.len(), rel.y.atan2(rel.x), 4.0 * b.atan())
    }

    /// The octant byte `±0SC`: (clockwise, start octant, octants with 0 = 8).
    fn octants(spec: u8) -> (f64, f64, f64) {
        let sign = if spec & 0x80 != 0 { -1.0 } else { 1.0 };
        let s = f64::from((spec >> 4) & 7);
        let c = match spec & 7 {
            0 => 8.0,
            c => f64::from(c),
        };
        (sign, s, c)
    }

    /// The vector unit size of an extended big font's subshape cell.
    fn cell(&self) -> Point {
        let h = if self.font.above() > 0.0 { self.font.above() } else { 1.0 };
        let w = if self.font.char_width() > 0.0 { self.font.char_width() } else { h };
        Point::new(w, h)
    }

    fn run(&mut self, number: u16, depth: usize) -> Res {
        if depth >= MAX_DEPTH {
            return Err(Broken);
        }
        let program = self.font.program(number).ok_or(Broken)?;
        let mut r = Cur { b: program, pos: 0 };
        let mut skip = false;
        while r.pos < r.b.len() {
            self.step()?;
            let code = r.byte()?;
            // Code 14: in a dual-orientation font the next command is for vertical text only.
            let exec = !skip;
            skip = false;
            match code {
                0 => return Ok(()),
                1 if exec => self.down = true,
                2 if exec => {
                    self.down = false;
                    self.flush();
                }
                3 | 4 => {
                    let k = f64::from(r.byte()?);
                    if exec && k > 0.0 {
                        let s = if code == 3 { self.scale / k } else { self.scale * k };
                        if !(1e-9..=MAX_COORD).contains(&s) {
                            return Err(Broken);
                        }
                        self.scale = s;
                    }
                }
                5 if exec => {
                    if self.stack.len() >= MAX_STACK {
                        return Err(Broken);
                    }
                    self.stack.push(self.pos);
                }
                6 if exec => {
                    self.flush();
                    if let Some(p) = self.stack.pop() {
                        self.pos = p;
                    }
                }
                7 => self.subshape(&mut r, exec, depth)?,
                8 | 9 | 12 | 13 => loop {
                    self.step()?;
                    let d = Point::new(signed(r.byte()?), signed(r.byte()?));
                    if matches!(code, 9 | 13) && d == Point::ZERO {
                        break;
                    }
                    let bulge = if matches!(code, 12 | 13) { signed(r.byte()?) } else { 0.0 };
                    if exec {
                        self.bulge(d, bulge)?;
                    }
                    if matches!(code, 8 | 12) {
                        break;
                    }
                },
                10 => {
                    let radius = f64::from(r.byte()?);
                    let (sign, s, c) = Self::octants(r.byte()?);
                    if exec {
                        self.arc(radius, s * FRAC_PI_4, sign * c * FRAC_PI_4)?;
                    }
                }
                11 => {
                    let start = f64::from(r.byte()?) / 256.0;
                    let end = match r.byte()? {
                        0 => 1.0,
                        e => f64::from(e) / 256.0,
                    };
                    let radius = f64::from(u16::from_be_bytes([r.byte()?, r.byte()?]));
                    let (sign, s, c) = Self::octants(r.byte()?);
                    if exec {
                        // From `start` past the first octant boundary to `end` past the last
                        // octant's.
                        self.arc(radius, (s + sign * start) * FRAC_PI_4, sign * (c - 1.0 + end - start) * FRAC_PI_4)?;
                    }
                }
                14 => skip = self.font.dual() && !self.vertical && exec,
                1..=15 => {}
                v => {
                    if exec {
                        self.by(direction(v) * f64::from(v >> 4))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Code 7 and its operands.
    fn subshape(&mut self, r: &mut Cur, exec: bool, depth: usize) -> Res {
        match self.font.kind() {
            ShxKind::Unifont => {
                let n = u16::from_be_bytes([r.byte()?, r.byte()?]);
                if exec {
                    self.run(n, depth + 1)?;
                }
            }
            ShxKind::BigFont => {
                let first = r.byte()?;
                if first != 0 {
                    if exec {
                        self.run(u16::from(first), depth + 1)?;
                    }
                    return Ok(());
                }
                // Extended: the primitive drawn in the box (x, y, width, height) of the cell.
                let n = u16::from_be_bytes([r.byte()?, r.byte()?]);
                let (x, y, w, h) = (f64::from(r.byte()?), f64::from(r.byte()?), f64::from(r.byte()?), f64::from(r.byte()?));
                if !exec {
                    return Ok(());
                }
                let (pos, axes, down) = (self.pos, self.axes, self.down);
                let cell = self.cell();
                self.flush();
                self.pos = pos + Point::new(x * axes.x, y * axes.y) * self.scale;
                self.axes = Point::new(axes.x * w / cell.x, axes.y * h / cell.y);
                let res = self.run(n, depth + 1);
                self.flush();
                self.axes = axes;
                self.pos = pos;
                self.down = down;
                res?;
            }
            ShxKind::Shapes => {
                let n = u16::from(r.byte()?);
                if exec {
                    self.run(n, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

impl Font for crate::ParsedFont {
    fn kind(&self) -> ShxKind {
        self.kind
    }
    fn dual(&self) -> bool {
        self.dual
    }
    fn above(&self) -> f64 {
        self.above
    }
    fn char_width(&self) -> f64 {
        self.char_width
    }
    fn program(&self, number: u16) -> Option<&[u8]> {
        let r = self.records.get(&number)?;
        self.data.get(r.program.0..r.program.1)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn executes_shape_vectors_without_app_geometry_types() {
        let bytes = crate::compile("*1,3,A\n1,010,0\n").unwrap();
        let font = crate::parse::parse(&bytes).unwrap();
        let glyph = outline(&font, 1, false).unwrap();
        assert_eq!(glyph.strokes, vec![vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0)]]);
        assert_eq!(glyph.advance, Point::new(1.0, 0.0));
    }
    #[test]
    fn recursive_shapes_stop_at_the_depth_bound() {
        let bytes = crate::compile("*1,3,A\n7,1,0\n").unwrap();
        let font = crate::parse::parse(&bytes).unwrap();
        assert!(outline(&font, 1, false).is_err());
    }
}

#[cfg(test)]
mod instruction_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    struct Fixture {
        kind: ShxKind,
        programs: Vec<Vec<u8>>,
    }
    impl Font for Fixture {
        fn kind(&self) -> ShxKind {
            self.kind
        }
        fn dual(&self) -> bool {
            true
        }
        fn above(&self) -> f64 {
            10.0
        }
        fn char_width(&self) -> f64 {
            10.0
        }
        fn program(&self, number: u16) -> Option<&[u8]> {
            self.programs.get(usize::from(number)).map(Vec::as_slice)
        }
    }
    fn fixture(program: Vec<u8>) -> Fixture {
        Fixture { kind: ShxKind::Shapes, programs: vec![program] }
    }

    #[test]
    fn pen_state_scaling_and_location_stack_keep_separate_strokes() {
        let font = fixture(vec![4, 4, 0x10, 5, 2, 8, 2, 3, 6, 1, 3, 2, 0x14, 0]);
        let g = outline(&font, 0, false).unwrap();
        assert_eq!(g.strokes, vec![vec![Point::ZERO, Point::new(4.0, 0.0)], vec![Point::new(4.0, 0.0), Point::new(4.0, 2.0)]]);
        assert_eq!(g.advance, Point::new(4.0, 2.0));
    }

    #[test]
    fn vertical_only_commands_consume_operands_without_affecting_horizontal_text() {
        let font = fixture(vec![14, 8, 10, 20, 0x10, 0]);
        assert_eq!(outline(&font, 0, false).unwrap().advance, Point::new(1.0, 0.0));
        assert_eq!(outline(&font, 0, true).unwrap().advance, Point::new(11.0, 20.0));
    }

    #[test]
    fn displacement_and_bulge_sequences_end_at_their_chord_endpoint() {
        for (program, end_x) in
            [(vec![9, 3, 4, 253, 252, 0, 0, 0], 0.0), (vec![12, 10, 0, 127, 0], 10.0), (vec![13, 10, 0, 129, 246, 0, 0, 0, 0, 0], 0.0)]
        {
            let g = outline(&fixture(program), 0, false).unwrap();
            assert!(g.advance.y.abs() < 1e-10);
            assert!((g.advance.x - end_x).abs() < 1e-10);
        }
    }

    #[test]
    fn full_octant_arcs_close_and_fractional_arcs_keep_radius() {
        for program in [vec![10, 10, 0, 0], vec![10, 10, 128, 0], vec![11, 0, 0, 0, 10, 0, 0]] {
            let g = outline(&fixture(program), 0, false).unwrap();
            assert!(g.advance.len() < 1e-10);
            assert_eq!(g.strokes[0].len(), 65);
        }
    }

    #[test]
    fn subshape_operands_follow_font_kind_and_bigfont_restores_parent_position() {
        for kind in [ShxKind::Shapes, ShxKind::Unifont, ShxKind::BigFont] {
            let call = match kind {
                ShxKind::Unifont => vec![7, 0, 1, 0],
                ShxKind::BigFont => vec![7, 0, 0, 1, 2, 3, 20, 30, 0],
                _ => vec![7, 1, 0],
            };
            let font = Fixture { kind, programs: vec![call, vec![0x10, 0]] };
            let g = outline(&font, 0, false).unwrap();
            if kind == ShxKind::BigFont {
                assert_eq!(g.advance, Point::ZERO);
                assert_eq!(g.strokes[0], vec![Point::new(2.0, 3.0), Point::new(4.0, 3.0)]);
            } else {
                assert_eq!(g.advance, Point::new(1.0, 0.0));
            }
        }
    }

    #[test]
    fn malformed_operands_and_resource_exhaustion_return_errors() {
        for program in [vec![8, 1], vec![7, 99], vec![5; MAX_STACK + 1], vec![0x10; MAX_POINTS + 1], vec![4, 255, 4, 255, 4, 255]] {
            assert!(outline(&fixture(program), 0, false).is_err());
        }
        assert!(outline(&fixture(vec![0]), 1, false).is_err());
    }
}
