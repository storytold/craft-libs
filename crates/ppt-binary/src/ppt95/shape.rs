//! PowerPoint 95 shapes: the shape code of a geometry record (3009) as a DrawingML preset, and a
//! format record (3005) as fill, line and shadow, learned from files PowerPoint 95 wrote and the
//! slides it rendered from them.
//!
//! The shape code is the [MS-ODRAW] shape type less 4, so the [`geom`] table names its preset; the
//! adjust value after it is a coordinate in the shape's 21600-unit square, as OfficeArt's are.
//!
//! A format record holds the line (+0 0xFF for none, +4 its colour), the fill (+12 0xFF for none,
//! +20 its colour) and the shadow (+24 its colour, +28 0x00 when on and 0xFF when off), then the
//! angle at +48 and a horizontal flip at +52. Each colour is R, G, B and an index: 0xFE for the RGB
//! itself, else a slot of the colour scheme.

use craft_pptx::style::Shadow;
use craft_pptx::{ColorRef, Fill, Geom, Line, Rgba};

use super::{RGB_GIVEN, Rec};
use crate::convert::Scheme;
use crate::geom;

/// The width of every line PowerPoint 95 drew in the probes' renderings, in points (8 master
/// units); no probe set another, so the format's width field is not known yet.
const LINE_WIDTH: f64 = 1.0;
/// How far a shadow falls right and down, in points. The probes turned shadows on and off but
/// never moved one, so the offset is not read: this is the 48 units (6 pt) every probe's format
/// holds at +36 and +40, where the offset most likely is.
const SHADOW_OFFSET: f64 = 6.0;

/// How a shape is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Look {
    pub fill: Fill,
    pub line: Line,
    pub shadow: Option<Shadow>,
    pub flip_h: bool,
}

impl Look {
    /// Nothing drawn: what a shape with no format record shows.
    pub(super) fn none() -> Look {
        Look { fill: Fill::None, line: Line::none(), shadow: None, flip_h: false }
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

/// A line of `color` as PowerPoint 95 draws it.
pub(super) fn line_of(color: Rgba) -> Line {
    Line::solid(ColorRef::rgb(color), LINE_WIDTH)
}

/// A format record as fill, line, shadow and flip. A part whose flag or colour is cut off or names
/// no scheme slot is not drawn.
pub(super) fn look(format: &Rec, scheme: &Scheme) -> Look {
    let on = |flag: usize, at: usize| format.u8(flag).filter(|&f| f != 0xFF).and_then(|_| color(format, at, scheme));
    let shadow = format.u8(28).filter(|&f| f == 0).and_then(|_| color(format, 24, scheme)).map(|c| Shadow {
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
    Look {
        fill: on(12, 20).map_or(Fill::None, |c| Fill::solid(ColorRef::rgb(c))),
        line: on(0, 4).map_or_else(Line::none, line_of),
        shadow,
        flip_h: format.u8(52).is_some_and(|f| f != 0),
    }
}

/// The preset for a shape code with its adjust value, in a box `w` × `h` points, and whether it is
/// drawn upside down from the preset. `None` for a code no probe showed.
///
/// An OfficeArt adjust value is a coordinate in the shape's 21600-unit square, stretched to the box
/// on each axis; a DrawingML one is a 100000 fraction of the width (the triangle's apex) or of the
/// shorter side. PowerPoint 95's trapezoid is wide at the top, DrawingML's at the bottom.
pub(super) fn preset(code: i8, adjust: Option<i32>, w: f64, h: f64) -> Option<(Geom, bool)> {
    if !(-3..=8).contains(&code) {
        return None;
    }
    let kind = u16::try_from(i16::from(code) + 4).ok()?;
    let name = geom::preset_name(kind)?;
    let fraction = |v: i32, max: f64| f64::from(v).clamp(0.0, max) / 21600.0 * 100_000.0;
    // A horizontal inset: a share of the width, as a share of the shorter side.
    let ss = w.min(h);
    let across = |v: i32, max: f64| if ss > 0.0 && w.is_finite() { fraction(v, max) * w / ss } else { fraction(v, max) };
    let adj = adjust.and_then(|v| match kind {
        2 => Some(geom::round_rect_adjust(v)),
        5 => Some(fraction(v, 21600.0)),
        7 => Some(across(v, 21600.0)),
        8 | 9 => Some(across(v, 10800.0)),
        10 | 11 => Some(fraction(v, 10800.0)),
        _ => None,
    });
    Some((Geom::Preset { name: name.into(), adj: adj.into_iter().collect() }, kind == 8))
}
