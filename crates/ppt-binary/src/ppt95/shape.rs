//! PowerPoint 95 shapes: the shape code of a geometry record (3009) as a DrawingML preset, and a
//! format record (3005) as fill, line and shadow, learned from files PowerPoint 95 wrote and the
//! slides it rendered from them.
//!
//! The shape code is the low byte of the geometry's first word, the [MS-ODRAW] shape type less 4,
//! so the [`geom`] table names its preset: −3 to 8 are the object model's shapes 1 to 12, and 9
//! to 14 its shapes 13 to 24 (an arrow and a thick arrow, which turn and mirror into the other
//! three directions, a home plate, a cube, a balloon and a seal). The adjust value after it is a
//! coordinate in the shape's 21600-unit square, as OfficeArt's are. A code no probe showed is a
//! rectangle.
//!
//! A format record holds the line (+0 0xFF for none, +1 its dash style, +3 its style less one,
//! which sets its width, +4 its colour), the fill (+12 0xFF for none, +20 its colour) and the
//! shadow (+28 0x00 when on and 0xFF when off, +29 2 when semi-transparent, +30 its colour, +36 and
//! +40 how far it falls right and down in master units), then the angle the user set at +48 and
//! the flips the user set at +52. Each colour is R, G, B and an index: 0xFE for the RGB itself,
//! else a slot of the colour scheme.
//!
//! Angles are in sixteenths of a degree and run counter-clockwise, where DrawingML's run clockwise.
//! The geometry record holds the outline as drawn: for a shape code of 0 and up, the byte above it
//! holds whole counter-clockwise quarter-turns in bits 4 and 5 and a horizontal mirror, made after
//! the turn, in bit 6; its own angle is what is left over, and its box is the outline's turned
//! box. That is all a shape needs ([`orient_drawn`]), the user's angle and flips included. The
//! rectangle, rounded rectangle and ellipse (codes −3 to −1, whose byte above is the sign) and
//! placeholders take their angle from the format, and the quarter-turns from the difference
//! ([`orient`]); flips don't change their outlines. A line's 3007 atom holds its arrowheads
//! ([`arrowheads`]). A freeform's points become a custom path in its box ([`freeform_path`]).

use craft_pptx::style::{Dash, LineEnd, Shadow};
use craft_pptx::{ColorRef, ColorTransform, CustomPath, Fill, Geom, Line, Rgba, Xfrm};

use super::{MU, RGB_GIVEN, Rec};
use crate::convert::Scheme;
use crate::geom;

/// The widths of line styles 1 to 6 (the format holds the style less one), in points: the pens
/// PowerPoint 95 drew them with were 8, 16, 32, 48, 64 and 80 master units wide.
const LINE_WIDTHS: [f64; 6] = [1.0, 2.0, 4.0, 6.0, 8.0, 10.0];
/// How far a shadow falls right and down without a full format, in master units: 6 pt, the
/// default every new shape has.
const SHADOW_OFFSET: i32 = 48;
/// The farthest a shadow falls on either axis, in master units (10 in, wider than any slide), so a
/// damaged offset can't send it off to infinity.
const MAX_SHADOW_OFFSET: i32 = 5760;
/// A semi-transparent shadow's opacity, in DrawingML's 100000ths.
const SEMI_TRANSPARENT: i32 = 50_000;
/// A whole turn and a quarter-turn in sixteenths of a degree.
const TURN: i64 = 360 * 16;
const QUARTER: i64 = TURN / 4;

/// How a shape is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Look {
    pub fill: Fill,
    pub line: Line,
    pub shadow: Option<Shadow>,
}

impl Look {
    /// Nothing drawn: what a shape with no format record shows.
    pub(super) fn none() -> Look {
        Look { fill: Fill::None, line: Line::none(), shadow: None }
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

/// A format record as fill, line and shadow (the angle and flips are read with the box,
/// [`orient`] and [`orient_drawn`]). A part whose flag or colour is cut off or names no scheme
/// slot is not drawn.
pub(super) fn look(format: &Rec, scheme: &Scheme) -> Look {
    let on = |flag: usize, at: usize| format.u8(flag).filter(|&f| f != 0xFF).and_then(|_| color(format, at, scheme));
    let shadow = format.u8(28).filter(|&f| f == 0).and_then(|_| color(format, 30, scheme)).map(|c| shadow_of(format, c));
    Look {
        fill: on(12, 20).map_or(Fill::None, |c| Fill::solid(ColorRef::rgb(c))),
        line: on(0, 4).map_or_else(Line::none, |c| line_of(c, Some(format))),
        shadow,
    }
}

/// A shadow of `color` falling as far as the format's +36 (right) and +40 (down) say, in master
/// units, semi-transparent when +29 is 2: a distance and a direction clockwise from the right.
fn shadow_of(format: &Rec, color: Rgba) -> Shadow {
    let offset = |at: usize| f64::from(format.i32(at).unwrap_or(SHADOW_OFFSET).clamp(-MAX_SHADOW_OFFSET, MAX_SHADOW_OFFSET)) / MU;
    let (dx, dy) = (offset(36), offset(40));
    let color = ColorRef::rgb(color);
    Shadow {
        color: if format.u8(29) == Some(2) { color.with(ColorTransform::Alpha(SEMI_TRANSPARENT)) } else { color },
        blur: 0.0,
        dist: dx.hypot(dy),
        dir: dy.atan2(dx).to_degrees().rem_euclid(360.0),
        inner: false,
        sx: 1.0,
        sy: 1.0,
        kx: 0.0,
        ky: 0.0,
        align: String::new(),
        rotate_with_shape: false,
    }
}

/// Swap a box's width and height about its centre: from a box turned a quarter to the unturned.
fn turn_back(xfrm: &mut Xfrm) {
    let (cx, cy) = (xfrm.x + xfrm.w / 2.0, xfrm.y + xfrm.h / 2.0);
    (xfrm.w, xfrm.h) = (xfrm.h, xfrm.w);
    (xfrm.x, xfrm.y) = (cx - xfrm.w / 2.0, cy - xfrm.h / 2.0);
}

/// The quarter-turns clockwise, 0 to 3, that take the unturned box `xfrm` gives (after [`orient`] or
/// [`orient_drawn`]) to the box the geometry record holds: `xfrm`'s angle less the geometry's own,
/// `drawn`, which the record's box doesn't include.
pub(super) fn quarter_turns(xfrm: &Xfrm, drawn: i32) -> usize {
    let cw = (xfrm.rot - clockwise(i64::from(drawn))).rem_euclid(360.0);
    // 0 to 4 quarters, 4 being a whole turn.
    [0, 1, 2, 3, 0][((cw / 90.0).round() as usize).min(4)]
}

/// A counter-clockwise angle in sixteenths of a degree as DrawingML's clockwise degrees, 0 to 360.
pub(super) fn clockwise(ccw: i64) -> f64 {
    ((-ccw).rem_euclid(TURN) as f64) / 16.0
}

/// Give `xfrm`, the geometry record's box, the counter-clockwise angle `turn` the user set, given
/// the angle `drawn` the geometry record holds (both in sixteenths of a degree): for shapes whose
/// geometry has no orientation bits. The quarter-turns between the two are folded into the drawn
/// box: after an odd number of them it is the turned box, so it is turned back about its centre.
pub(super) fn orient(xfrm: &mut Xfrm, drawn: i32, turn: i32) {
    let quarters = (i64::from(turn) - i64::from(drawn) + QUARTER / 2).div_euclid(QUARTER).rem_euclid(4);
    if quarters % 2 == 1 {
        turn_back(xfrm);
    }
    xfrm.rot = clockwise(i64::from(turn));
}

/// Give `xfrm`, the geometry record's box, the transform the orientation bits `bits` (the byte
/// above a shape code of 0 and up) and the geometry's own angle `drawn` say: the preset turned
/// `(bits >> 4) & 3` quarter-turns counter-clockwise, then mirrored across when bit 0x40 is set,
/// then turned by `drawn`, counter-clockwise, about the box's centre. The box is the outline's
/// after the quarter-turns, so after an odd number of them it is turned back.
///
/// DrawingML flips first and then turns clockwise. Without the mirror that is no flip and the
/// whole angle negated. With it, a mirror after a counter-clockwise turn by `q` is the same as a
/// clockwise turn by `q` after the mirror, so it is a horizontal flip, turned clockwise by `q`
/// less `drawn`.
pub(super) fn orient_drawn(xfrm: &mut Xfrm, bits: u8, drawn: i32) {
    let quarters = i64::from((bits >> 4) & 3) * QUARTER;
    let mirror = bits & 0x40 != 0;
    if bits & 0x10 != 0 {
        turn_back(xfrm);
    }
    let ccw = if mirror { i64::from(drawn) - quarters } else { quarters + i64::from(drawn) };
    xfrm.rot = clockwise(ccw);
    (xfrm.flip_h, xfrm.flip_v) = (mirror, false);
}

/// PowerPoint 95's trapezoid flipped top to bottom, as its default is drawn: wide at the top, each
/// bottom corner inset by the adjust value (0 to half the side, a quarter by default). The outline
/// the PowerPoint 97 path gives OfficeArt's trapezoid; a negative adjust value is 0, as there.
fn trapezoid_top(adjust: Option<i32>) -> Geom {
    geom::trapezoid(adjust.map(|v| u32::try_from(v).unwrap_or(0)))
}

/// A freeform's points as a path in its box `sides` (left, top, right and bottom, either way
/// round), all in master units from the slide's centre: a move to the first point, lines to the
/// rest, and a close when `closed`. The path's size is the box's (at least one unit each way, so a
/// flat freeform still has a coordinate space), its points measured from the box's top left.
pub(super) fn freeform_path(points: &[(i32, i32)], [l, t, r, b]: [i32; 4], closed: bool) -> Geom {
    let (left, top) = (f64::from(l.min(r)), f64::from(t.min(b)));
    let (w, h) = ((f64::from(r) - f64::from(l)).abs().max(1.0), (f64::from(b) - f64::from(t)).abs().max(1.0));
    let mut d = String::new();
    for (i, &(x, y)) in points.iter().enumerate() {
        let cmd = if i == 0 { "M" } else { " L" };
        d.push_str(&format!("{cmd} {} {}", f64::from(x) - left, f64::from(y) - top));
    }
    if closed {
        d.push_str(" Z");
    }
    Geom::Custom { paths: vec![CustomPath { w, h, d, ..Default::default() }] }
}

/// The share of its square a balloon's body takes, from the top; the tail reaches the bottom.
const BALLOON_BODY: f64 = 5.0 / 6.0;

/// The geometry for a shape code with its adjust value, in `xfrm`, the box already turned and
/// flipped ([`orient`], [`orient_drawn`]): a preset, or the trapezoid's own outline. A code no
/// probe showed is a rectangle.
///
/// An OfficeArt adjust value is a coordinate in the shape's 21600-unit square, stretched to the box
/// on each axis; a DrawingML one is a 100000 fraction of the width (the triangle's apex), the
/// height or the shorter side. Where PowerPoint 95 has no adjust value for a part, the proportion
/// is the one it drew: the arrow's shaft is half the height, the thick arrow's three quarters.
///
/// Two shapes change `xfrm`. A mirrored trapezoid (its default) is the outline wide at the top,
/// unflipped and turned a half-turn further: a horizontal flip is a vertical one turned a
/// half-turn, and the vertical one is in the outline, so its text stays upright. The balloon's box
/// is its body only, as DrawingML's callout's is, its tail reaching out below ([`balloon`]).
pub(super) fn geometry(code: i8, adjust: Option<i32>, xfrm: &mut Xfrm) -> Geom {
    let kind = u16::try_from(i16::from(code) + 4).ok().filter(|_| (-3..=14).contains(&code));
    // PowerPoint 95's trapezoid unmirrored is DrawingML's preset, wide at the bottom; the PowerPoint
    // 97 path's table has no preset for it, OfficeArt's being wide at the top.
    let Some(name) = kind.and_then(|k| if k == 8 { Some("trapezoid") } else { geom::preset_name(k) }) else {
        log::warn!("ppt: PowerPoint 95 shape code {code} is not known; read as a rectangle");
        return Geom::Preset { name: "rect".into(), adj: vec![] };
    };
    let (w, h) = (xfrm.w, xfrm.h);
    let fraction = |v: i32, max: i32| f64::from(v.clamp(0, max)) / 21600.0 * 100_000.0;
    // A horizontal length: a share of the width, as a share of the shorter side.
    let ss = w.min(h);
    let across = |v: i32, max: i32| if ss > 0.0 && w.is_finite() { fraction(v, max) * w / ss } else { fraction(v, max) };
    // The length from a horizontal coordinate to the right side.
    let to_right = |v: i32| across(21600 - v.clamp(0, 21600), 21600);
    let adj: Vec<f64> = match (kind, adjust) {
        (Some(8), _) if xfrm.flip_h => {
            xfrm.flip_h = false;
            xfrm.rot = (xfrm.rot + 180.0) % 360.0;
            return trapezoid_top(adjust);
        }
        (Some(13), _) => vec![50_000.0, to_right(adjust.unwrap_or(10800))],
        (Some(14), _) => vec![75_000.0, to_right(adjust.unwrap_or(10800))],
        (Some(17), _) => return balloon(adjust, xfrm),
        (_, None) => vec![],
        (Some(2), Some(v)) => vec![geom::round_rect_adjust(v)],
        (Some(5 | 16), Some(v)) => vec![fraction(v, 21600)],
        (Some(7), Some(v)) => vec![across(v, 21600)],
        (Some(8 | 9), Some(v)) => vec![across(v, 10800)],
        (Some(10 | 11), Some(v)) => vec![fraction(v, 10800)],
        (Some(15), Some(v)) => vec![to_right(v)],
        _ => vec![],
    };
    Geom::Preset { name: name.into(), adj }
}

/// The balloon: a rounded body over the top five sixths of its square, with a tail from its bottom
/// edge to the square's bottom at the adjust value across (1800, a twelfth, by default). As in
/// DrawingML's callout, its box becomes the body, moved along the shape's own vertical axis so the
/// body stays where it was drawn, and the tail's tip is given from the body's centre.
fn balloon(adjust: Option<i32>, xfrm: &mut Xfrm) -> Geom {
    let tip = f64::from(adjust.unwrap_or(1800).clamp(0, 21600)) / 21600.0;
    let (w, h) = (xfrm.w, xfrm.h);
    let body = h * BALLOON_BODY;
    // The body's centre is (h − body) / 2 above the box's, on the shape's own axis, turned with it.
    let (sin, cos) = xfrm.rot.to_radians().sin_cos();
    let up = (h - body) / 2.0;
    let (cx, cy) = (xfrm.x + w / 2.0 + up * sin, xfrm.y + h / 2.0 - up * cos);
    (xfrm.x, xfrm.y, xfrm.h) = (cx - w / 2.0, cy - body / 2.0, body);
    // The corners are rounded a sixth of the square's shorter side.
    let ss = w.min(body);
    let radius = if ss > 0.0 && ss.is_finite() { w.min(h) / 6.0 / ss * 100_000.0 } else { 16_667.0 };
    let below = (1.0 / BALLOON_BODY - 0.5) * 100_000.0;
    Geom::Preset { name: "wedgeRoundRectCallout".into(), adj: vec![(tip - 0.5) * 100_000.0, below, radius] }
}
