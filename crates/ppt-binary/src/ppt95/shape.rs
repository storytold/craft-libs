//! PowerPoint 95 shapes: the shape code of a geometry record (3009) as a DrawingML preset, and a
//! format record (3005) as fill, line, shadow, angle and flips, learned from files PowerPoint 95
//! wrote and the slides it rendered from them.
//!
//! The shape code is the [MS-ODRAW] shape type less 4, so the [`geom`] table names its preset; the
//! adjust value after it is a coordinate in the shape's 21600-unit square, as OfficeArt's are.
//!
//! A format record holds the line (+0 0xFF for none, +1 its dash style, +3 its style less one,
//! which sets its width, +4 its colour), the fill (+12 0xFF for none, +20 its colour) and the
//! shadow (+28 0x00 when on and 0xFF when off, +30 its colour), then the angle the user set at +48
//! (sixteenths of a degree) and the flips at +52 (bit 1 horizontal, bit 2 vertical). Each colour is
//! R, G, B and an index: 0xFE for the RGB itself, else a slot of the colour scheme.
//!
//! The geometry record holds the outline as drawn: whole quarter-turns of the angle are folded into
//! orientation bits above its shape code, its box and its own angle, which is only what is left
//! over. So the angle comes from the format, and the box is turned back by the quarter-turns
//! between the two ([`orient`]). A line's 3007 atom holds its arrowheads ([`arrowheads`]).

use craft_pptx::style::{Dash, LineEnd, Shadow};
use craft_pptx::{ColorRef, CustomPath, Fill, Geom, Line, Rgba, Xfrm};

use super::{RGB_GIVEN, Rec};
use crate::convert::Scheme;
use crate::geom;

/// The widths of line styles 1 to 6 (the format holds the style less one), in points: the pens
/// PowerPoint 95 drew them with were 8, 16, 32, 48, 64 and 80 master units wide.
const LINE_WIDTHS: [f64; 6] = [1.0, 2.0, 4.0, 6.0, 8.0, 10.0];
/// How far a shadow falls right and down, in points. The offset can't be set through the object
/// model, so no probe moved it: this is the 48 units (6 pt) every probe's format holds at +36 and
/// +40, where the offset most likely is, and they are not read.
const SHADOW_OFFSET: f64 = 6.0;
/// A whole turn in sixteenths of a degree.
const TURN: i64 = 360 * 16;

/// How a shape is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Look {
    pub fill: Fill,
    pub line: Line,
    pub shadow: Option<Shadow>,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Look {
    /// Nothing drawn: what a shape with no format record shows.
    pub(super) fn none() -> Look {
        Look { fill: Fill::None, line: Line::none(), shadow: None, flip_h: false, flip_v: false }
    }

    /// Whether the shape draws anything of its own (its text aside).
    pub(super) fn drawn(&self) -> bool {
        self.fill != Fill::None || self.line != Line::none() || self.shadow.is_some()
    }
}

/// A format record's colour at `at`: R, G, B, then 0xFE or a slot of `scheme`.
pub(super) fn color(format: &Rec, at: usize, scheme: &Scheme) -> Option<Rgba> {
    let (r, g, b, index) = (format.u8(at)?, format.u8(at + 1)?, format.u8(at + 2)?, format.u8(at + 3)?);
    if index == RGB_GIVEN { Some(Rgba::rgb(r, g, b)) } else { scheme.get(usize::from(index)).copied() }
}

/// A line of `color` in the style and dashes of `format`: 1 pt and solid without one, or for a
/// style or dash value no probe showed.
pub(super) fn line_of(color: Rgba, format: Option<&Rec>) -> Line {
    let width = format.and_then(|f| f.u8(3)).and_then(|s| LINE_WIDTHS.get(usize::from(s))).copied().unwrap_or(1.0);
    let mut line = Line::solid(ColorRef::rgb(color), width);
    let dash = match format.and_then(|f| f.u8(1)) {
        Some(1) => Some("sysDot"),
        Some(2) => Some("dash"),
        Some(3) => Some("lgDash"),
        Some(4) => Some("dashDot"),
        _ => None,
    };
    line.dash = dash.map(|name| Dash::Preset { name: name.into() });
    line
}

/// A line's arrowheads from its 3007 atom: +0 the start's, +1 the end's. 4 is a small swept-back
/// head, 1 a longer and wider one; 0, and any value no probe showed, is none.
pub(super) fn arrowheads(line: &mut Line, atom: &Rec) {
    let end = |v: Option<u8>| {
        let (kind, size) = match v? {
            4 => ("stealth", "med"),
            1 => ("triangle", "lg"),
            _ => return None,
        };
        Some(LineEnd { kind: kind.into(), w: size.into(), len: size.into() })
    };
    (line.head, line.tail) = (end(atom.u8(0)), end(atom.u8(1)));
}

/// A format record as fill, line, shadow and flips (the angle is read with the box, [`orient`]). A part whose flag or colour is cut off
/// or names no scheme slot is not drawn.
pub(super) fn look(format: &Rec, scheme: &Scheme) -> Look {
    let on = |flag: usize, at: usize| format.u8(flag).filter(|&f| f != 0xFF).and_then(|_| color(format, at, scheme));
    let shadow = format.u8(28).filter(|&f| f == 0).and_then(|_| color(format, 30, scheme)).map(|c| Shadow {
        color: ColorRef::rgb(c),
        blur: 0.0,
        dist: SHADOW_OFFSET.hypot(SHADOW_OFFSET),
        dir: 45.0,
        inner: false,
        sx: 1.0,
        sy: 1.0,
        kx: 0.0,
        ky: 0.0,
        align: String::new(),
        rotate_with_shape: false,
    });
    let flips = format.u8(52).unwrap_or(0);
    Look {
        fill: on(12, 20).map_or(Fill::None, |c| Fill::solid(ColorRef::rgb(c))),
        line: on(0, 4).map_or_else(Line::none, |c| line_of(c, Some(format))),
        shadow,
        flip_h: flips & 1 != 0,
        flip_v: flips & 2 != 0,
    }
}

/// Give `xfrm`, the geometry record's box, the angle `turn` the user set, given the angle `drawn`
/// the geometry record holds (both in sixteenths of a degree). The quarter-turns between them are
/// folded into the drawn box: after an odd number of them it is the turned box, so its width and
/// height are swapped back about its centre.
pub(super) fn orient(xfrm: &mut Xfrm, drawn: i32, turn: i32) {
    let quarter = TURN / 4;
    let quarters = (i64::from(turn) - i64::from(drawn) + quarter / 2).div_euclid(quarter).rem_euclid(4);
    if quarters % 2 == 1 {
        let (cx, cy) = (xfrm.x + xfrm.w / 2.0, xfrm.y + xfrm.h / 2.0);
        (xfrm.w, xfrm.h) = (xfrm.h, xfrm.w);
        (xfrm.x, xfrm.y) = (cx - xfrm.w / 2.0, cy - xfrm.h / 2.0);
    }
    xfrm.rot = (i64::from(turn).rem_euclid(TURN) as f64) / 16.0;
}

/// PowerPoint 95's trapezoid: wide at the top, each bottom corner inset by the adjust value (0 to
/// half the side, a quarter by default) in the shape's 21600-unit square. The same outline the
/// PowerPoint 97 path gives the trapezoid (storytold/craft-libs#2492), to be shared once that lands.
fn trapezoid(adjust: Option<i32>) -> Geom {
    let a = adjust.unwrap_or(5400).clamp(0, 10800);
    let d = format!("M 0 0 L {a} 21600 L {} 21600 L 21600 0 Z", 21600 - a);
    Geom::Custom { paths: vec![CustomPath { w: 21600.0, h: 21600.0, d, ..Default::default() }] }
}

/// The geometry for a shape code with its adjust value, in a box `w` × `h` points: a preset, or
/// the trapezoid's own outline. `None` for a code no probe showed.
///
/// An OfficeArt adjust value is a coordinate in the shape's 21600-unit square, stretched to the box
/// on each axis; a DrawingML one is a 100000 fraction of the width (the triangle's apex) or of the
/// shorter side.
pub(super) fn preset(code: i8, adjust: Option<i32>, w: f64, h: f64) -> Option<Geom> {
    if !(-3..=8).contains(&code) {
        return None;
    }
    let kind = u16::try_from(i16::from(code) + 4).ok()?;
    if kind == 8 {
        return Some(trapezoid(adjust));
    }
    let name = geom::preset_name(kind)?;
    let fraction = |v: i32, max: f64| f64::from(v).clamp(0.0, max) / 21600.0 * 100_000.0;
    // A horizontal inset: a share of the width, as a share of the shorter side.
    let ss = w.min(h);
    let across = |v: i32, max: f64| if ss > 0.0 && w.is_finite() { fraction(v, max) * w / ss } else { fraction(v, max) };
    let adj = adjust.and_then(|v| match kind {
        2 => Some(geom::round_rect_adjust(v)),
        5 => Some(fraction(v, 21600.0)),
        7 => Some(across(v, 21600.0)),
        9 => Some(across(v, 10800.0)),
        10 | 11 => Some(fraction(v, 10800.0)),
        _ => None,
    });
    Some(Geom::Preset { name: name.into(), adj: adj.into_iter().collect() })
}
