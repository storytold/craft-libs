//! EMF and WMF writers (feature `write`).
//!
//! The writers take a list of drawing operations ([`Op`]) in paint order, in points with y down,
//! and the region of that space the picture covers: its top-left corner is the picture's origin
//! and its size the picture's size. What an application's document holds beyond these operations
//! (gradients, patterns, type, effects, transparency of paths) is turned into them by the
//! application.
//!
//! - **EMF** ([`emf`]): paths keep their Bézier curves; fills are solid brushes; strokes are
//!   geometric pens with their width, cap, join, miter limit and dashes; clips are path clips
//!   inside a saved device context; images keep their pixels and their transparency and may be
//!   turned or skewed. The header's frame is the region in 0.01 mm.
//! - **WMF** ([`wmf`]): the same operations with curves flattened into polygons and 16-bit
//!   coordinates, behind a placeable header that gives the size. WMF has no clip paths, no
//!   transparency and no turned images; [`Wmf`] says which of these the picture lost.
//!
//! The reader reads back what the writers write: the tests round-trip both formats through
//! [`crate::parse`].

mod dib;
mod emf;
mod wmf;

use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl, Point, Rect};

use crate::{Cap, Join};

/// Most line segments one curve is flattened into, whatever its size and the tolerance.
const MAX_FLATTEN_PER_CURVE: f64 = 4096.0;
/// A curve whose control polygon is longer than this, or has a point further out, is written as
/// a line: kurbo squares and cubes coordinates when it sizes its work, which overflows there.
const MAX_FLATTEN_LENGTH: f64 = 1e60;
/// Most dashes one stroke is cut into when its pen cannot draw them (the rest of the path is
/// then left solid, so a tiny pattern on a long path cannot make an endless file).
const MAX_DASHES: usize = 100_000;

/// One drawing operation.
#[derive(Clone, Debug)]
pub enum Op {
    /// A path filled with one colour.
    Fill {
        path: BezPath,
        even_odd: bool,
        rgb: [u8; 3],
    },
    /// A path stroked with a pen.
    Stroke {
        path: BezPath,
        pen: Pen,
    },
    /// An image: `xf` maps its pixel space (`0..width`, `0..height`) into the picture space; its
    /// alpha is multiplied by `opacity`.
    Image {
        image: Arc<Image>,
        xf: Affine,
        opacity: f32,
    },
    /// What follows, up to the matching [`Op::Unclip`], is clipped to `path` (inside the clips
    /// around it). WMF has no clip paths and writes what follows whole.
    Clip {
        path: BezPath,
        even_odd: bool,
    },
    Unclip,
}

/// A stroke as a pen draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    pub rgb: [u8; 3],
    /// Points.
    pub width: f64,
    pub cap: Cap,
    pub join: Join,
    /// Longest miter as a multiple of the width (for [`Join::Miter`]).
    pub miter: f64,
    pub dash: Option<Dash>,
}

/// A dash pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct Dash {
    /// Dash, gap, dash, gap… in points. An odd count repeats to an even one.
    pub pattern: Vec<f64>,
    /// How far into the pattern the path starts.
    pub offset: f64,
}

impl Dash {
    /// Whether the pattern draws dashes: every length finite and not negative, and not all zero.
    fn is_dashed(&self) -> bool {
        self.offset.is_finite() && self.pattern.iter().all(|v| v.is_finite() && *v >= 0.0) && self.pattern.iter().any(|v| *v > 0.0)
    }

    /// Whether an EMF pen draws this pattern itself (`PS_USERSTYLE`): it starts at the path's
    /// start, has at most 16 entries once made even, and every entry is at least one device unit
    /// (0.01 mm). Other patterns are written as one line per dash, cut with [`kurbo::dash`]; a
    /// caller that cuts dashes differently (aligned to corners, say) asks here and passes the
    /// dashes as the path with no pattern.
    pub fn emf_pen_draws(&self) -> bool {
        emf::dash_entries(self).is_some()
    }
}

/// Pixels with straight (not premultiplied) alpha, RGBA8, row by row from the top.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// Every pixel fully opaque?
    pub fn opaque(&self) -> bool {
        self.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255)
    }
}

/// A written WMF and what the format could not hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wmf {
    pub bytes: Vec<u8>,
    /// Images with transparency were written over white.
    pub translucent_images: bool,
    /// Turned or skewed images were written upright, filling their bounds.
    pub turned_images: bool,
    /// Clips were not applied: what they cover was written whole.
    pub clips_ignored: bool,
    /// Images with a side over 32,767 pixels were left out.
    pub oversized_images: bool,
}

/// Writes `ops` over `region` as an EMF. The header's description names the application and the
/// picture (`app`, `title`). None when the region is empty or not finite.
pub fn emf(ops: &[Op], region: Rect, app: &str, title: &str) -> Option<Vec<u8>> {
    Some(emf::write(ops, sane(region)?, app, title))
}

/// Writes `ops` over `region` as a placeable WMF. None when the region is empty or not finite.
pub fn wmf(ops: &[Op], region: Rect) -> Option<Wmf> {
    Some(wmf::write(ops, sane(region)?))
}

/// `region` with its corners in order, when it is finite and has an area.
fn sane(region: Rect) -> Option<Rect> {
    let r = region.abs();
    ([r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) && r.width() > 0.0 && r.height() > 0.0).then_some(r)
}

/// `bp` cut into its dashes (as it is when `dash` draws none), for pens that can't draw them.
fn dashed(bp: &BezPath, dash: Option<&Dash>) -> BezPath {
    let Some(d) = dash.filter(|d| d.is_dashed()) else { return bp.clone() };
    let mut out = BezPath::new();
    let mut dashes = 0usize;
    for el in kurbo::dash(bp.iter(), d.offset, &d.pattern) {
        dashes += usize::from(matches!(el, PathEl::MoveTo(_)));
        if dashes > MAX_DASHES {
            // Too many to write one by one: the stroke stays solid.
            return bp.clone();
        }
        out.push(el);
    }
    out
}

/// `bp` flattened to within `tol`: each subpath's points and whether it closes.
fn flatten(bp: &BezPath, tol: f64) -> Vec<(Vec<Point>, bool)> {
    let mut out: Vec<(Vec<Point>, bool)> = vec![];
    let mut cur: Vec<Point> = vec![];
    flatten_capped(bp.iter(), tol, |el| match el {
        PathEl::MoveTo(p) => {
            if cur.len() > 1 {
                out.push((std::mem::take(&mut cur), false));
            }
            cur = vec![p];
        }
        PathEl::LineTo(p) => cur.push(p),
        PathEl::ClosePath => {
            if cur.len() > 1 {
                out.push((std::mem::take(&mut cur), true));
            }
            cur.clear();
        }
        _ => {}
    });
    if cur.len() > 1 {
        out.push((cur, false));
    }
    out
}

/// [`kurbo::flatten`], each curve's tolerance raised so that it makes at most about
/// [`MAX_FLATTEN_PER_CURVE`] segments. kurbo sizes its work and a buffer from the curve's
/// coordinates before producing anything, so a small curve with far-out handles asks for
/// gigabytes. The segments a curve needs grow with the square root of its control polygon's
/// length over the tolerance; that ratio is held under the square of the cap. A curve whose
/// points aren't finite is drawn as a line to its end.
fn flatten_capped(path: impl IntoIterator<Item = PathEl>, tolerance: f64, mut callback: impl FnMut(PathEl)) {
    let mut last = None;
    let mut start = None;
    for el in path {
        let (from, to, length) = match el {
            PathEl::MoveTo(p) => {
                (last, start) = (Some(p), Some(p));
                callback(el);
                continue;
            }
            PathEl::LineTo(p) => {
                last = Some(p);
                callback(el);
                continue;
            }
            PathEl::ClosePath => {
                last = start;
                callback(el);
                continue;
            }
            PathEl::QuadTo(a, b) => (last, b, last.map(|p| p.distance(a) + a.distance(b))),
            PathEl::CurveTo(a, b, c) => (last, c, last.map(|p| p.distance(a) + a.distance(b) + b.distance(c))),
        };
        last = Some(to);
        let (Some(from), Some(length)) = (from, length) else { continue };
        // A short polygon far from the origin still overflows kurbo's `p * (mt * 2.0)` products.
        let far = |p: Point| !(p.x.abs() <= MAX_FLATTEN_LENGTH && p.y.abs() <= MAX_FLATTEN_LENGTH);
        let controls_far = match el {
            PathEl::QuadTo(a, _) => far(a),
            PathEl::CurveTo(a, b, _) => far(a) || far(b),
            _ => false,
        };
        if length.is_nan() || length > MAX_FLATTEN_LENGTH || far(from) || far(to) || controls_far {
            callback(PathEl::LineTo(to));
            continue;
        }
        let tol = tolerance.max(length / (MAX_FLATTEN_PER_CURVE * MAX_FLATTEN_PER_CURVE));
        kurbo::flatten([PathEl::MoveTo(from), el], tol, |e| {
            if !matches!(e, PathEl::MoveTo(_)) {
                callback(e);
            }
        });
    }
}

/// Little-endian bytes being written.
#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    /// Zeros up to the next multiple of `n` bytes.
    fn pad(&mut self, n: usize) {
        while !self.0.len().is_multiple_of(n) {
            self.0.push(0);
        }
    }
    /// Overwrite the `u32` at `at` (a size or count known only later).
    fn set_u32(&mut self, at: usize, v: u32) {
        if let Some(b) = self.0.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }
    fn set_i32(&mut self, at: usize, v: i32) {
        if let Some(b) = self.0.get_mut(at..at + 4) {
            b.copy_from_slice(&v.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests;
