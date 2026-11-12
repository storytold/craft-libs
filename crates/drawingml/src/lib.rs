//! DrawingML (ECMA-376 part 1, chapter 20): the parts of it that PresentationML,
//! WordprocessingML and SpreadsheetML drawings share.
//!
//! - Units and placement: EMU conversion ([`emu_to_pt`], [`pt_to_emu`]) and `a:xfrm` ([`Xfrm`],
//!   [`group_child_affine`]). Lengths in this crate are points (1/72 inch) as `f64`; files store
//!   EMU (12 700 per point), and the conversion is exact for integer EMU values.
//! - [`color`]: sRGB colours, theme colour slots, the colour transforms (`a:lumMod`, `a:tint`,
//!   `a:alpha` …) and the preset and system colour names.
//! - [`preset`]: outlines of the preset shapes (`a:prstGeom`), by their file names.
//! - [`custgeom`]: custom geometry (`a:custGeom`), with the shape guide formulas.
//! - [`warp`]: preset text warps (`a:prstTxWarp`).
//! - The model: [`style`] (colour references, fills, lines, effects), [`text`] (text bodies,
//!   paragraphs, runs), [`table`], [`chart`] with [`numfmt`] (number format codes), [`theme`]
//!   (colour, font and format schemes) and [`diagram`] (SmartArt outlines). Media are named by
//!   [`MediaId`]; the bytes stay with the caller.
//! - Readers and writers: [`read`] and [`write`] (`a:` fills, lines, effects, `a:xfrm`, text
//!   bodies), [`chart::read`], [`chart::chartex`] and [`chart::write`] (`c:chartSpace`,
//!   `cx:chartSpace`), [`theme::read_theme`] and [`theme::theme_xml`], [`table::styles`]
//!   (`a:tblStyleLst`) and [`diagram::parts`] (`dgm:` data, layout, style and colour parts).
//!   What they need from the package is the caller's [`part::Source`] or [`part::Sink`];
//!   [`xmlx`] holds the parser settings and markup-compatibility handling they share.
//!
//! Nothing here opens a package: readers take a [`craft_xml::El`] and writers a
//! [`craft_xml::Writer`], so the same code serves a slide, a document or a worksheet drawing.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod chart;
pub mod color;
pub mod custgeom;
pub mod diagram;
pub mod numfmt;
pub mod part;
pub mod preset;
pub mod read;
pub mod style;
pub mod table;
pub mod text;
pub mod theme;
pub mod write;
pub mod xmlx;

/// Namespace URIs.
pub mod ns {
    pub const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
    pub const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    pub const NS_A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";
    pub const NS_C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
}
mod trendline;
pub mod warp;

use kurbo::{Affine, Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

pub(crate) fn yes() -> bool {
    true
}

pub(crate) fn some_yes() -> Option<bool> {
    Some(true)
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u32);
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}
id_type!(
    /// A picture, sound or other embedded file, by the id its owner gave it. The bytes stay with
    /// the owner (a package part in a file).
    MediaId
);
id_type!(
    /// The target of a link to a slide. Only presentations have them.
    SlideId
);

/// EMU per point.
pub const EMU_PER_PT: f64 = 12_700.0;
/// EMU per inch.
pub const EMU_PER_IN: f64 = 914_400.0;
/// EMU per centimetre.
pub const EMU_PER_CM: f64 = 360_000.0;

pub fn emu_to_pt(emu: i64) -> f64 {
    emu as f64 / EMU_PER_PT
}

/// Points to EMU, rounded. Non-finite input becomes 0; huge values saturate.
pub fn pt_to_emu(pt: f64) -> i64 {
    if !pt.is_finite() {
        return 0;
    }
    let v = (pt * EMU_PER_PT).round();
    if v >= i64::MAX as f64 {
        i64::MAX
    } else if v <= i64::MIN as f64 {
        i64::MIN
    } else {
        v as i64
    }
}

/// A shape's placement on its page (or in its group's child space): offset, extent, rotation
/// (degrees clockwise) and flips. Rotation and flips are about the centre.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Xfrm {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub rot: f64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub flip_h: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub flip_v: bool,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl Xfrm {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Xfrm { x, y, w, h, ..Default::default() }
    }
    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.x + self.w, self.y + self.h)
    }
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    /// Local shape space (0..w, 0..h) → parent space, applying flips and rotation about the centre.
    pub fn affine(&self) -> Affine {
        let c = Vec2::new(self.w / 2.0, self.h / 2.0);
        let sx = if self.flip_h { -1.0 } else { 1.0 };
        let sy = if self.flip_v { -1.0 } else { 1.0 };
        Affine::translate(Vec2::new(self.x, self.y) + c)
            * Affine::rotate(self.rot.to_radians())
            * Affine::scale_non_uniform(sx, sy)
            * Affine::translate(-c)
    }
    /// Axis-aligned bounds of the rotated box in parent space.
    pub fn bounds(&self) -> Rect {
        let a = self.affine();
        let pts = [Point::new(0.0, 0.0), Point::new(self.w, 0.0), Point::new(self.w, self.h), Point::new(0.0, self.h)].map(|p| a * p);
        let mut r = Rect::from_points(pts[0], pts[1]);
        r = r.union_pt(pts[2]).union_pt(pts[3]);
        r
    }
    /// True when no dimension is NaN/inf.
    pub fn is_finite(&self) -> bool {
        [self.x, self.y, self.w, self.h, self.rot].iter().all(|v| v.is_finite())
    }
    /// Rotation normalised to 0..360.
    pub fn rot_norm(&self) -> f64 {
        let r = self.rot.rem_euclid(360.0);
        if r >= 360.0 { 0.0 } else { r }
    }
    /// The box `inner` occupies under `m` (inner space → parent), as an `a:xfrm`: rotated about
    /// its centre, flipped vertically for a negative determinant. `None` when `m` shears (an
    /// `a:xfrm` cannot), or is not finite. A shape drawn in `inner`'s space then maps onto the
    /// box by the per-axis scales of [`decompose`]`(m)`.
    pub fn from_affine(inner: Rect, m: Affine) -> Option<Xfrm> {
        let dm = decompose(m);
        if dm.shear.abs() > 1e-6 || ![dm.sx, dm.sy, dm.rot].iter().all(|v| v.is_finite()) {
            return None;
        }
        let c = m * inner.center();
        let (w, h) = (inner.width() * dm.sx.abs(), inner.height() * dm.sy.abs());
        let x = Xfrm { x: c.x - w / 2.0, y: c.y - h / 2.0, w, h, rot: dm.rot, flip_h: false, flip_v: dm.sy < 0.0 };
        x.is_finite().then_some(x)
    }
}

/// An affine split into rotation, axis scales and shear: `m = translate · rotate(rot) ·
/// [sx, shear·sy; 0, sy]`. `rot` is in degrees; `sy` is negative for a mirroring transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decomposed {
    pub rot: f64,
    pub sx: f64,
    pub sy: f64,
    pub shear: f64,
}

/// See [`Decomposed`].
pub fn decompose(m: Affine) -> Decomposed {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let sx = a.hypot(b);
    let rot = b.atan2(a);
    let (cos, sin) = (rot.cos(), rot.sin());
    let c2 = cos * c + sin * d;
    let d2 = -sin * c + cos * d;
    let shear = if d2.abs() > 1e-12 { c2 / d2 } else { 0.0 };
    Decomposed { rot: rot.to_degrees(), sx, sy: d2, shear }
}

/// Maps a group's child coordinate space (`ch_off`, `ch_ext`) onto the group's own box.
pub fn group_child_affine(group: &Xfrm, ch: Rect) -> Affine {
    let sx = if ch.width().abs() > 1e-9 { group.w / ch.width() } else { 1.0 };
    let sy = if ch.height().abs() > 1e-9 { group.h / ch.height() } else { 1.0 };
    group.affine() * Affine::scale_non_uniform(sx, sy) * Affine::translate(-ch.origin().to_vec2())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emu_roundtrip_is_exact() {
        for emu in [0i64, 1, 12_699, 12_700, 914_400, 12_192_000, 6_858_000, -5, 123_456_789] {
            assert_eq!(pt_to_emu(emu_to_pt(emu)), emu);
        }
        assert_eq!(pt_to_emu(f64::NAN), 0);
        assert_eq!(pt_to_emu(f64::INFINITY), 0);
        assert_eq!(pt_to_emu(1e300), i64::MAX);
        assert_eq!(pt_to_emu(-1e300), i64::MIN);
    }

    #[test]
    fn xfrm_affine_rotation_about_center() {
        let x = Xfrm { x: 10.0, y: 20.0, w: 100.0, h: 50.0, rot: 90.0, ..Default::default() };
        let c = x.affine() * Point::new(50.0, 25.0);
        assert!((c.x - 60.0).abs() < 1e-9 && (c.y - 45.0).abs() < 1e-9);
        let b = x.bounds();
        assert!((b.width() - 50.0).abs() < 1e-9 && (b.height() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn flips_mirror_local_space() {
        let x = Xfrm { x: 0.0, y: 0.0, w: 100.0, h: 50.0, flip_h: true, ..Default::default() };
        let p = x.affine() * Point::new(0.0, 0.0);
        assert!((p.x - 100.0).abs() < 1e-9 && p.y.abs() < 1e-9);
    }

    #[test]
    fn group_child_mapping() {
        let g = Xfrm::new(100.0, 100.0, 200.0, 100.0);
        let a = group_child_affine(&g, Rect::new(0.0, 0.0, 100.0, 50.0));
        let p = a * Point::new(100.0, 50.0);
        assert!((p.x - 300.0).abs() < 1e-9 && (p.y - 200.0).abs() < 1e-9);
    }

    #[test]
    fn rot_norm_stays_below_360() {
        for rot in [-1e-15, -1e-13, -f64::EPSILON, -0.0, 360.0, 720.0, -360.0] {
            let n = Xfrm { rot, ..Default::default() }.rot_norm();
            assert!((0.0..360.0).contains(&n), "rot {rot} -> {n}");
        }
    }

    #[test]
    fn an_affine_without_shear_becomes_an_xfrm_that_places_the_box_the_same_way() {
        let inner = Rect::new(10.0, 20.0, 110.0, 70.0);
        for (rot, sx, sy) in [(0.0, 1.0, 1.0), (30.0, 2.0, 0.5), (-120.0, 1.5, -2.0), (90.0, 1.0, -1.0)] {
            let m = Affine::translate((300.0, 200.0)) * Affine::rotate(f64::to_radians(rot)) * Affine::scale_non_uniform(sx, sy);
            let x = Xfrm::from_affine(inner, m).unwrap();
            let dm = decompose(m);
            assert!((dm.rot - rot).abs() < 1e-9 && (dm.sx - sx).abs() < 1e-9 && (dm.sy - sy).abs() < 1e-9, "{dm:?}");
            assert_eq!(x.flip_v, sy < 0.0);
            // The xfrm's local box, scaled from inner space, lands where `m` puts each corner.
            let local = |p: Point| Point::new((p.x - inner.x0) * sx.abs(), (p.y - inner.y0) * sy.abs());
            for p in [Point::new(inner.x0, inner.y0), Point::new(inner.x1, inner.y0), Point::new(inner.x1, inner.y1)] {
                let (a, b) = (x.affine() * local(p), m * p);
                assert!((a - b).hypot() < 1e-9, "rot {rot} sy {sy}: {a:?} vs {b:?}");
            }
        }
        assert!(Xfrm::from_affine(inner, Affine::skew(0.3, 0.0)).is_none(), "a shear has no xfrm");
        assert!(Xfrm::from_affine(inner, Affine::scale(f64::NAN)).is_none());
    }
}
