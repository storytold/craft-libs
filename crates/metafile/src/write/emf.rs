//! The EMF writer: an EMR_HEADER whose frame is the region in 0.01 mm, then the drawing in device
//! units of 0.01 mm (the reference device has 100 pixels a millimetre) and an EMR_EOF.
//!
//! Each fill and stroke is a path bracket (move-to, poly-line-to and poly-Bézier-to runs, close
//! figure) filled with a solid brush or stroked with a geometric pen; brushes and pens are made
//! when the colour or pen changes and deleted when replaced. Clips are path clips inside a saved
//! DC; images are StretchDIBits (opaque) or AlphaBlend (with transparency) under a world transform
//! that places their pixels.

use kurbo::{Affine, BezPath, PathEl, Point, Rect};

use super::{Dash, Image, Op, Out, Pen, dib};
use crate::{Cap, Join};

// Record types (MS-EMF 2.1.1).
pub(crate) const HEADER: u32 = 1;
pub(crate) const POLYBEZIERTO: u32 = 5;
pub(crate) const POLYLINETO: u32 = 6;
pub(crate) const EOF: u32 = 14;
pub(crate) const SETBKMODE: u32 = 18;
pub(crate) const SETPOLYFILLMODE: u32 = 19;
pub(crate) const MOVETOEX: u32 = 27;
pub(crate) const SAVEDC: u32 = 33;
pub(crate) const RESTOREDC: u32 = 34;
pub(crate) const SETWORLDTRANSFORM: u32 = 35;
pub(crate) const MODIFYWORLDTRANSFORM: u32 = 36;
pub(crate) const SELECTOBJECT: u32 = 37;
pub(crate) const CREATEBRUSHINDIRECT: u32 = 39;
pub(crate) const DELETEOBJECT: u32 = 40;
pub(crate) const SETMITERLIMIT: u32 = 58;
pub(crate) const BEGINPATH: u32 = 59;
pub(crate) const ENDPATH: u32 = 60;
pub(crate) const CLOSEFIGURE: u32 = 61;
pub(crate) const FILLPATH: u32 = 62;
pub(crate) const STROKEPATH: u32 = 64;
pub(crate) const SELECTCLIPPATH: u32 = 67;
pub(crate) const STRETCHDIBITS: u32 = 81;
pub(crate) const EXTCREATEPEN: u32 = 95;
pub(crate) const ALPHABLEND: u32 = 114;

/// The `" EMF"` signature in the header.
pub(crate) const SIGNATURE: u32 = 0x464D_4520;
/// Stock objects (selected to release ours before deleting them).
const WHITE_BRUSH: u32 = 0x8000_0000;
const NULL_PEN: u32 = 0x8000_0008;
/// Our two object slots (0 is reserved).
const BRUSH: u32 = 1;
const PEN: u32 = 2;

pub(crate) const PS_GEOMETRIC: u32 = 0x0001_0000;
pub(crate) const PS_USERSTYLE: u32 = 7;
pub(crate) const PS_ENDCAP_SQUARE: u32 = 0x100;
pub(crate) const PS_ENDCAP_FLAT: u32 = 0x200;
pub(crate) const PS_JOIN_BEVEL: u32 = 0x1000;
pub(crate) const PS_JOIN_MITER: u32 = 0x2000;
/// Raster operation: copy the source.
pub(crate) const SRCCOPY: u32 = 0x00CC_0020;

/// Device units (0.01 mm) per point.
pub(crate) const UNITS_PER_PT: f64 = 2540.0 / 72.0;
/// Longest dash pattern a geometric pen takes (longer ones are drawn as separate dashes).
const MAX_STYLE: usize = 16;
/// Header size with both extensions.
const HEADER_SIZE: usize = 108;
/// Coordinates are kept within this (far beyond any page).
const MAX_COORD: f64 = 1e9;

/// A COLORREF (0x00bbggrr).
pub(crate) fn colorref(rgb: [u8; 3]) -> u32 {
    u32::from_le_bytes([rgb[0], rgb[1], rgb[2], 0])
}

/// The style entries (device units) of a pen that draws `d` itself; None when the dashes must be
/// written as separate lines: a pen's dashes start at the path's start, every entry is at least a
/// unit long, and there are 16 entries at most.
pub(super) fn dash_entries(d: &Dash) -> Option<Vec<u32>> {
    let mut v: Vec<f64> = d.pattern.clone();
    if v.len() % 2 == 1 {
        v.extend_from_within(..);
    }
    let e: Vec<u32> = v.iter().map(|x| (x * UNITS_PER_PT).round().clamp(0.0, f64::from(u32::MAX)) as u32).collect();
    (d.offset.abs() < 1e-9 && e.len() <= MAX_STYLE && e.iter().all(|x| *x >= 1)).then_some(e)
}

/// What a pen object is made of, to tell when the next stroke needs another.
#[derive(Clone, PartialEq)]
struct PenKey {
    style: u32,
    width: u32,
    color: u32,
    entries: Vec<u32>,
}

/// A device-space bounding box (inclusive).
type Bounds = [i32; 4];

fn union(a: Option<Bounds>, b: Bounds) -> Bounds {
    match a {
        None => b,
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
    }
}

struct Writer {
    out: Out,
    records: u32,
    /// Picture space → device units.
    to_dev: Affine,
    /// The frame in device units: what the region clip lets through.
    frame: Bounds,
    /// Everything drawn, in device units.
    bounds: Option<Bounds>,
    brush: Option<u32>,
    pen: Option<PenKey>,
    fill_mode: Option<u32>,
    miter: Option<u32>,
}

/// Write `ops` as an EMF of `region`; the header's description names `app` and `title`.
pub(crate) fn write(ops: &[Op], region: Rect, app: &str, title: &str) -> Vec<u8> {
    let k = UNITS_PER_PT;
    let frame = [0, 0, (region.width() * k).round() as i32, (region.height() * k).round() as i32];
    let mut w = Writer {
        out: Out::default(),
        records: 0,
        to_dev: Affine::scale(k) * Affine::translate(-region.origin().to_vec2()),
        frame,
        bounds: None,
        brush: None,
        pen: None,
        fill_mode: None,
        miter: None,
    };
    w.header(app, title);
    // Dash gaps and hatches show what is below.
    w.record(SETBKMODE, |o| o.u32(1));
    for op in ops {
        w.op(op);
    }
    w.release();
    w.record(EOF, |o| {
        o.u32(0);
        o.u32(16);
        o.u32(20);
    });
    w.finish()
}

impl Writer {
    /// Append one record: its type, size and `body`, padded to 4 bytes.
    fn record(&mut self, kind: u32, body: impl FnOnce(&mut Out)) {
        let start = self.out.len();
        self.out.u32(kind);
        self.out.u32(0);
        body(&mut self.out);
        self.out.pad(4);
        let size = u32::try_from(self.out.len() - start).unwrap_or(u32::MAX);
        self.out.set_u32(start + 4, size);
        self.records += 1;
    }

    fn header(&mut self, app: &str, title: &str) {
        // "Application\0Picture\0\0", each part cut to what a description may reasonably hold.
        let part = |s: &str| s.chars().filter(|c| *c != '\0').take(200).collect::<String>();
        let desc: Vec<u16> = part(app).encode_utf16().chain([0]).chain(part(title).encode_utf16()).chain([0, 0]).collect();
        let f = self.frame;
        // The reference device: 100 pixels a millimetre, as large as the frame.
        let mm = [(f[2] / 100).max(1) + 1, (f[3] / 100).max(1) + 1];
        self.record(HEADER, |o| {
            // Bounds, patched in `finish`.
            o.bytes(&[0; 16]);
            for v in f {
                o.i32(v);
            }
            o.u32(SIGNATURE);
            o.u32(0x0001_0000);
            // Bytes, records and handles, patched in `finish`.
            o.u32(0);
            o.u32(0);
            o.u16(3);
            o.u16(0);
            o.u32(desc.len() as u32);
            o.u32(HEADER_SIZE as u32);
            o.u32(0);
            o.i32(mm[0] * 100);
            o.i32(mm[1] * 100);
            o.i32(mm[0]);
            o.i32(mm[1]);
            // No pixel format, not OpenGL; the device in micrometres.
            o.u32(0);
            o.u32(0);
            o.u32(0);
            o.u32(mm[0] as u32 * 1000);
            o.u32(mm[1] as u32 * 1000);
            for c in &desc {
                o.u16(*c);
            }
        });
    }

    /// Patch the header's bounds, size and record count.
    fn finish(mut self) -> Vec<u8> {
        let b = self.bounds.map(|b| [b[0].max(self.frame[0]), b[1].max(self.frame[1]), b[2].min(self.frame[2]), b[3].min(self.frame[3])]);
        // An empty picture has bounds (0, 0, -1, -1).
        let b = b.filter(|b| b[0] <= b[2] && b[1] <= b[3]).unwrap_or([0, 0, -1, -1]);
        for (i, v) in b.iter().enumerate() {
            self.out.set_i32(8 + 4 * i, *v);
        }
        let size = u32::try_from(self.out.len()).unwrap_or(u32::MAX);
        self.out.set_u32(48, size);
        self.out.set_u32(52, self.records);
        self.out.0
    }

    fn dev(&self, p: Point) -> [i32; 2] {
        let q = self.to_dev * p;
        let c = |v: f64| if v.is_finite() { v.round().clamp(-MAX_COORD, MAX_COORD) as i32 } else { 0 };
        [c(q.x), c(q.y)]
    }

    fn grow(&mut self, b: Bounds) {
        self.bounds = Some(union(self.bounds, b));
    }

    fn op(&mut self, op: &Op) {
        match op {
            Op::Fill { path, even_odd, rgb } => {
                self.fill_mode(*even_odd);
                self.brush(*rgb);
                if let Some(b) = self.path(path) {
                    self.grow(b);
                    self.record(FILLPATH, |o| b.iter().for_each(|v| o.i32(*v)));
                }
            }
            Op::Stroke { path, pen } => {
                let dashed;
                let path = if self.pen(pen) {
                    path
                } else {
                    dashed = super::dashed(path, pen.dash.as_ref());
                    &dashed
                };
                if let Some(b) = self.path(path) {
                    let half = (pen.width * UNITS_PER_PT / 2.0).ceil() as i32;
                    let b = [b[0] - half, b[1] - half, b[2] + half, b[3] + half];
                    self.grow(b);
                    self.record(STROKEPATH, |o| b.iter().for_each(|v| o.i32(*v)));
                }
            }
            Op::Image { image, xf, opacity } => self.image(image, *xf, *opacity),
            Op::Clip { path, even_odd } => {
                self.release();
                self.record(SAVEDC, |_| {});
                self.fill_mode(*even_odd);
                if self.path(path).is_some() {
                    // RGN_AND: inside the clip already set.
                    self.record(SELECTCLIPPATH, |o| o.u32(1));
                }
            }
            Op::Unclip => {
                self.release();
                self.record(RESTOREDC, |o| o.i32(-1));
                // The DC's fill mode and miter limit are back to what they were at the save.
                self.fill_mode = None;
                self.miter = None;
            }
        }
    }

    fn fill_mode(&mut self, even_odd: bool) {
        // ALTERNATE (even-odd) or WINDING (non-zero).
        let mode = if even_odd { 1 } else { 2 };
        if self.fill_mode != Some(mode) {
            self.record(SETPOLYFILLMODE, |o| o.u32(mode));
            self.fill_mode = Some(mode);
        }
    }

    /// Select a solid brush of `rgb`, made when the colour changes.
    fn brush(&mut self, rgb: [u8; 3]) {
        let color = colorref(rgb);
        if self.brush == Some(color) {
            return;
        }
        self.drop_brush();
        self.record(CREATEBRUSHINDIRECT, |o| {
            o.u32(BRUSH);
            // BS_SOLID, the colour, no hatch.
            o.u32(0);
            o.u32(color);
            o.u32(0);
        });
        self.record(SELECTOBJECT, |o| o.u32(BRUSH));
        self.brush = Some(color);
    }

    fn drop_brush(&mut self) {
        if self.brush.take().is_some() {
            self.record(SELECTOBJECT, |o| o.u32(WHITE_BRUSH));
            self.record(DELETEOBJECT, |o| o.u32(BRUSH));
        }
    }

    fn drop_pen(&mut self) {
        if self.pen.take().is_some() {
            self.record(SELECTOBJECT, |o| o.u32(NULL_PEN));
            self.record(DELETEOBJECT, |o| o.u32(PEN));
        }
    }

    /// Release our objects (before a DC is saved or restored, which would bring back deleted ones).
    fn release(&mut self) {
        self.drop_brush();
        self.drop_pen();
    }

    /// Select a geometric pen for `p`, made when it changes. False when `p`'s dashes must be
    /// drawn as separate lines (the pen selected is solid): a pen's dashes start at the path's
    /// start, every entry at least a unit long, 16 entries at most.
    fn pen(&mut self, p: &Pen) -> bool {
        let k = UNITS_PER_PT;
        let entries = p.dash.as_ref().map(dash_entries);
        let native = !matches!(entries, Some(None));
        let entries = entries.flatten().unwrap_or_default();
        let cap = match p.cap {
            Cap::Flat => PS_ENDCAP_FLAT,
            Cap::Round => 0,
            Cap::Square => PS_ENDCAP_SQUARE,
        };
        let join = match p.join {
            Join::Miter => PS_JOIN_MITER,
            Join::Round => 0,
            Join::Bevel => PS_JOIN_BEVEL,
        };
        let style = PS_GEOMETRIC | cap | join | if entries.is_empty() { 0 } else { PS_USERSTYLE };
        let key = PenKey { style, width: (p.width * k).round().clamp(1.0, 1e9) as u32, color: colorref(p.rgb), entries };
        if p.join == Join::Miter {
            let m = p.miter.round().clamp(1.0, 1000.0) as u32;
            if self.miter != Some(m) {
                self.record(SETMITERLIMIT, |o| o.u32(m));
                self.miter = Some(m);
            }
        }
        if self.pen.as_ref() != Some(&key) {
            self.drop_pen();
            let n = key.entries.len() as u32;
            self.record(EXTCREATEPEN, |o| {
                o.u32(PEN);
                // No pattern bitmap: its offsets point past the record, its sizes are 0.
                let end = 52 + 4 * n;
                o.u32(end);
                o.u32(0);
                o.u32(end);
                o.u32(0);
                o.u32(key.style);
                o.u32(key.width);
                // BS_SOLID, the colour, no hatch.
                o.u32(0);
                o.u32(key.color);
                o.u32(0);
                o.u32(n);
                key.entries.iter().for_each(|e| o.u32(*e));
            });
            self.record(SELECTOBJECT, |o| o.u32(PEN));
            self.pen = Some(key);
        }
        native
    }

    /// `bp` as a path bracket → its device bounds (`None`: nothing to draw).
    fn path(&mut self, bp: &BezPath) -> Option<Bounds> {
        if !bp.elements().iter().any(|e| !matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath)) {
            return None;
        }
        self.record(BEGINPATH, |_| {});
        let mut bounds: Option<Bounds> = None;
        let mut run: Vec<[i32; 2]> = vec![];
        let mut curves = false;
        let mut last = Point::ZERO;
        let mut start = Point::ZERO;
        for el in bp.elements() {
            let (pts, is_curve): (Vec<Point>, bool) = match *el {
                PathEl::MoveTo(p) => {
                    self.flush_run(&mut run, curves);
                    let d = self.dev(p);
                    bounds = Some(union(bounds, [d[0], d[1], d[0], d[1]]));
                    self.record(MOVETOEX, |o| d.iter().for_each(|v| o.i32(*v)));
                    (last, start) = (p, p);
                    continue;
                }
                PathEl::ClosePath => {
                    self.flush_run(&mut run, curves);
                    self.record(CLOSEFIGURE, |_| {});
                    last = start;
                    continue;
                }
                PathEl::LineTo(p) => (vec![p], false),
                PathEl::QuadTo(c, p) => (vec![last + (c - last) * (2.0 / 3.0), p + (c - p) * (2.0 / 3.0), p], true),
                PathEl::CurveTo(a, b, p) => (vec![a, b, p], true),
            };
            if is_curve != curves {
                self.flush_run(&mut run, curves);
                curves = is_curve;
            }
            for p in &pts {
                let d = self.dev(*p);
                bounds = Some(union(bounds, [d[0], d[1], d[0], d[1]]));
                run.push(d);
            }
            last = pts.last().copied().unwrap_or(last);
        }
        self.flush_run(&mut run, curves);
        self.record(ENDPATH, |_| {});
        bounds
    }

    /// Write the points of a run of lines or curves, then empty it.
    fn flush_run(&mut self, run: &mut Vec<[i32; 2]>, curves: bool) {
        if run.is_empty() {
            return;
        }
        let pts = std::mem::take(run);
        let b = pts.iter().fold(None, |acc, p| Some(union(acc, [p[0], p[1], p[0], p[1]]))).unwrap_or([0, 0, -1, -1]);
        self.record(if curves { POLYBEZIERTO } else { POLYLINETO }, |o| {
            b.iter().for_each(|v| o.i32(*v));
            o.u32(pts.len() as u32);
            for p in &pts {
                o.i32(p[0]);
                o.i32(p[1]);
            }
        });
    }

    /// An image placed by `xf` (its pixels → picture space): StretchDIBits when opaque, else AlphaBlend,
    /// drawn at its pixel size under a world transform.
    fn image(&mut self, img: &Image, xf: Affine, opacity: f32) {
        if img.width == 0 || img.height == 0 || opacity <= 0.0 {
            return;
        }
        let m = self.to_dev * xf;
        if !m.as_coeffs().iter().all(|v| v.is_finite()) || m.determinant().abs() < 1e-12 {
            return;
        }
        let (w, h) = (f64::from(img.width), f64::from(img.height));
        let corners = [Point::ZERO, Point::new(w, 0.0), Point::new(0.0, h), Point::new(w, h)].map(|p| self.dev(xf * p));
        let b = corners.iter().fold(None, |acc, p| Some(union(acc, [p[0], p[1], p[0], p[1]]))).unwrap_or([0, 0, -1, -1]);
        self.grow(b);
        let [a, bb, c, d, e, f] = m.as_coeffs();
        self.record(SETWORLDTRANSFORM, |o| [a, bb, c, d, e, f].iter().for_each(|v| o.f32(*v as f32)));
        let (iw, ih) = (img.width as i32, img.height as i32);
        if img.opaque() && opacity >= 0.998 {
            let (bmi, bits) = dib::rgb24(img, 1.0);
            self.record(STRETCHDIBITS, |o| {
                b.iter().for_each(|v| o.i32(*v));
                // Destination and source rectangles.
                [0, 0, 0, 0, iw, ih].iter().for_each(|v| o.i32(*v));
                o.u32(80);
                o.u32(bmi.len() as u32);
                o.u32(80 + bmi.len() as u32);
                o.u32(bits.len() as u32);
                // DIB_RGB_COLORS, SRCCOPY, the destination size.
                o.u32(0);
                o.u32(SRCCOPY);
                o.i32(iw);
                o.i32(ih);
                o.bytes(&bmi);
                o.bytes(&bits);
            });
        } else {
            let (bmi, bits) = dib::bgra32(img, opacity);
            self.record(ALPHABLEND, |o| {
                b.iter().for_each(|v| o.i32(*v));
                [0, 0, iw, ih].iter().for_each(|v| o.i32(*v));
                // AC_SRC_OVER, no flags, constant alpha 255, AC_SRC_ALPHA (per-pixel alpha).
                o.bytes(&[0, 0, 255, 1]);
                o.i32(0);
                o.i32(0);
                // The source's own transform: identity.
                [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0].iter().for_each(|v| o.f32(*v));
                o.u32(0);
                o.u32(0);
                o.u32(108);
                o.u32(bmi.len() as u32);
                o.u32(108 + bmi.len() as u32);
                o.u32(bits.len() as u32);
                o.i32(iw);
                o.i32(ih);
                o.bytes(&bmi);
                o.bytes(&bits);
            });
        }
        // MWT_IDENTITY: back to drawing in device units.
        self.record(MODIFYWORLDTRANSFORM, |o| {
            [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0].iter().for_each(|v| o.f32(*v));
            o.u32(1);
        });
    }
}
