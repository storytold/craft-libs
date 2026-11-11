//! The WMF writer: a placeable header (the picture's size and logical units an inch, with its
//! checksum), the WMF header, then the drawing in 16-bit logical units with an anisotropic window
//! the size of the picture, and the end-of-file record.
//!
//! Curves are flattened into polygons (fills) and polylines (strokes, with pens of the stroke's
//! width, caps and joins; dashes as separate lines). Objects take the lowest free slot of the
//! object table, as players number them: slots 0 and 1 hold a null pen and a null brush, our pen
//! and brush come after and are deleted when replaced. Images are StretchDIB records of 24-bit
//! bitmaps composited over white.

use kurbo::{Affine, BezPath, Point, Rect};

use super::emf::{PS_ENDCAP_FLAT, PS_ENDCAP_SQUARE, PS_JOIN_BEVEL, PS_JOIN_MITER, SRCCOPY, colorref};
use super::{Image, Op, Out, Pen, Wmf, dib};
use crate::{Cap, Join};

/// The key a placeable header starts with.
pub(crate) const PLACEABLE_KEY: u32 = 0x9AC6_CDD7;

// Record functions (MS-WMF 2.1.1.1).
pub(crate) const EOF: u16 = 0x0000;
pub(crate) const SETBKMODE: u16 = 0x0102;
pub(crate) const SETMAPMODE: u16 = 0x0103;
pub(crate) const SETPOLYFILLMODE: u16 = 0x0106;
pub(crate) const SETWINDOWORG: u16 = 0x020B;
pub(crate) const SETWINDOWEXT: u16 = 0x020C;
pub(crate) const SELECTOBJECT: u16 = 0x012D;
pub(crate) const DELETEOBJECT: u16 = 0x01F0;
pub(crate) const CREATEPENINDIRECT: u16 = 0x02FA;
pub(crate) const CREATEBRUSHINDIRECT: u16 = 0x02FC;
pub(crate) const POLYGON: u16 = 0x0324;
pub(crate) const POLYLINE: u16 = 0x0325;
pub(crate) const POLYPOLYGON: u16 = 0x0538;
pub(crate) const STRETCHDIB: u16 = 0x0F43;

/// Most logical units an inch (twips, what office apps use).
const MAX_INCH: f64 = 1440.0;
/// The largest 16-bit coordinate.
const MAX_I16: f64 = 32767.0;
/// Most points a polygon or polyline record holds (its count is 16-bit).
const MAX_POINTS: usize = 32767;
/// Curves are flattened to within this many logical units.
const FLATNESS: f64 = 0.25;
/// The null pen's and the null brush's slots.
const NULL_PEN: u16 = 0;
const NULL_BRUSH: u16 = 1;

/// The logical units an inch for a picture `w` × `h` points: the most, up to 1440, that keep its
/// size within 16 bits.
pub(crate) fn units_per_inch(w: f64, h: f64) -> u16 {
    let side_in = w.max(h) / 72.0;
    let fit = if side_in > 0.0 { (MAX_I16 / side_in).floor() } else { MAX_INCH };
    fit.clamp(1.0, MAX_INCH) as u16
}

/// The XOR of the first ten 16-bit words of a placeable header.
pub(crate) fn checksum(header: &[u8]) -> u16 {
    header.as_chunks::<2>().0.iter().take(10).fold(0, |acc, w| acc ^ u16::from_le_bytes(*w))
}

/// What a pen object is made of.
#[derive(Clone, Copy, PartialEq)]
struct PenKey {
    style: u16,
    width: i16,
    color: u32,
}

struct Writer {
    /// The records after the headers.
    out: Out,
    max_record: u32,
    to_dev: Affine,
    /// Logical units per point.
    k: f64,
    /// Which object slots are taken.
    slots: Vec<bool>,
    brush: Option<(u16, u32)>,
    pen: Option<(u16, PenKey)>,
    sel_pen: u16,
    sel_brush: u16,
    fill_mode: Option<u16>,
}

/// Write `ops` (curves flattened, no clips) as a placeable WMF of `region`.
pub(crate) fn write(ops: &[Op], region: Rect) -> Wmf {
    let inch = units_per_inch(region.width(), region.height());
    let k = f64::from(inch) / 72.0;
    let size = |v: f64| (v * k).round().clamp(1.0, MAX_I16) as i16;
    let (w, h) = (size(region.width()), size(region.height()));
    let mut wr = Writer {
        out: Out::default(),
        max_record: 0,
        to_dev: Affine::scale(k) * Affine::translate(-region.origin().to_vec2()),
        k,
        slots: vec![],
        brush: None,
        pen: None,
        sel_pen: NULL_PEN,
        sel_brush: NULL_BRUSH,
        fill_mode: None,
    };
    // MM_ANISOTROPIC, a window the size of the picture, transparent background.
    wr.record(SETMAPMODE, |o| o.u16(8));
    wr.record(SETWINDOWORG, |o| {
        o.i16(0);
        o.i16(0);
    });
    wr.record(SETWINDOWEXT, |o| {
        o.i16(h);
        o.i16(w);
    });
    wr.record(SETBKMODE, |o| o.u16(1));
    // The null pen and brush (slots 0 and 1), selected while ours change.
    wr.alloc();
    wr.record(CREATEPENINDIRECT, |o| {
        o.u16(5);
        o.i16(0);
        o.i16(0);
        o.u32(0);
    });
    wr.alloc();
    wr.record(CREATEBRUSHINDIRECT, |o| {
        o.u16(1);
        o.u32(0);
        o.u16(0);
    });
    wr.select(NULL_PEN);
    wr.select(NULL_BRUSH);
    let mut lost = Wmf::default();
    for op in ops {
        match op {
            Op::Fill { path, even_odd, rgb } => wr.fill(path, *even_odd, *rgb),
            Op::Stroke { path, pen } => wr.stroke(path, pen),
            Op::Image { image, xf, opacity } => {
                // The record's sizes are 16-bit.
                if image.width > MAX_POINTS as u32 || image.height > MAX_POINTS as u32 {
                    lost.oversized_images = true;
                    continue;
                }
                lost.translucent_images |= !image.opaque() || *opacity < 0.998;
                lost.turned_images |= !wr.image(image, *xf, *opacity);
            }
            // WMF has no clip paths: what a clip covers is written whole.
            Op::Clip { .. } => lost.clips_ignored = true,
            Op::Unclip => {}
        }
    }
    if let Some((slot, _)) = wr.brush.take() {
        wr.select(NULL_BRUSH);
        wr.delete(slot);
    }
    if let Some((slot, _)) = wr.pen.take() {
        wr.select(NULL_PEN);
        wr.delete(slot);
    }
    wr.record(EOF, |_| {});
    Wmf { bytes: wr.finish(w, h, inch), ..lost }
}

impl Writer {
    /// Append one record: its size in 16-bit words, its function and `body`.
    fn record(&mut self, function: u16, body: impl FnOnce(&mut Out)) {
        let start = self.out.len();
        self.out.u32(0);
        self.out.u16(function);
        body(&mut self.out);
        self.out.pad(2);
        let words = u32::try_from((self.out.len() - start) / 2).unwrap_or(u32::MAX);
        self.out.set_u32(start, words);
        self.max_record = self.max_record.max(words);
    }

    /// The placeable header, the WMF header, then the records.
    fn finish(self, w: i16, h: i16, inch: u16) -> Vec<u8> {
        let mut o = Out::default();
        o.u32(PLACEABLE_KEY);
        o.u16(0);
        for v in [0, 0, w, h] {
            o.i16(v);
        }
        o.u16(inch);
        o.u32(0);
        let sum = checksum(&o.0);
        o.u16(sum);
        // A memory metafile, 9 words of header, version 3.
        o.u16(1);
        o.u16(9);
        o.u16(0x0300);
        o.u32(u32::try_from((18 + self.out.len()) / 2).unwrap_or(u32::MAX));
        o.u16(u16::try_from(self.slots.len()).unwrap_or(u16::MAX));
        o.u32(self.max_record);
        o.u16(0);
        o.bytes(&self.out.0);
        o.0
    }

    /// The lowest free object slot, taken.
    fn alloc(&mut self) -> u16 {
        let i = match self.slots.iter().position(|t| !t) {
            Some(i) => i,
            None => {
                self.slots.push(false);
                self.slots.len() - 1
            }
        };
        if let Some(s) = self.slots.get_mut(i) {
            *s = true;
        }
        u16::try_from(i).unwrap_or(u16::MAX)
    }

    fn select(&mut self, slot: u16) {
        self.record(SELECTOBJECT, |o| o.u16(slot));
    }

    fn delete(&mut self, slot: u16) {
        self.record(DELETEOBJECT, |o| o.u16(slot));
        if let Some(s) = self.slots.get_mut(usize::from(slot)) {
            *s = false;
        }
    }

    fn dev(&self, p: Point) -> [i16; 2] {
        let q = self.to_dev * p;
        let c = |v: f64| if v.is_finite() { v.round().clamp(-MAX_I16, MAX_I16) as i16 } else { 0 };
        [c(q.x), c(q.y)]
    }

    /// The device points of `pts`, repeats dropped, thinned to what one record holds.
    fn points(&self, pts: &[Point]) -> Vec<[i16; 2]> {
        let mut v: Vec<[i16; 2]> = vec![];
        for p in pts {
            let d = self.dev(*p);
            if v.last() != Some(&d) {
                v.push(d);
            }
        }
        if v.len() > MAX_POINTS {
            let step = v.len().div_ceil(MAX_POINTS - 1);
            let last = v.last().copied();
            v = v.into_iter().step_by(step).collect();
            v.extend(last);
        }
        v
    }

    fn select_pen(&mut self, slot: u16) {
        if self.sel_pen != slot {
            self.select(slot);
            self.sel_pen = slot;
        }
    }

    fn select_brush(&mut self, slot: u16) {
        if self.sel_brush != slot {
            self.select(slot);
            self.sel_brush = slot;
        }
    }

    fn use_brush(&mut self, rgb: [u8; 3]) {
        let color = colorref(rgb);
        let slot = match self.brush {
            Some((slot, c)) if c == color => slot,
            old => {
                if let Some((slot, _)) = old {
                    self.select_brush(NULL_BRUSH);
                    self.delete(slot);
                }
                let slot = self.alloc();
                self.record(CREATEBRUSHINDIRECT, |o| {
                    o.u16(0);
                    o.u32(color);
                    o.u16(0);
                });
                self.brush = Some((slot, color));
                slot
            }
        };
        self.select_brush(slot);
    }

    fn use_pen(&mut self, p: &Pen) {
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
        let key = PenKey { style: (cap | join) as u16, width: (p.width * self.k).round().clamp(1.0, MAX_I16) as i16, color: colorref(p.rgb) };
        let slot = match self.pen {
            Some((slot, k)) if k == key => slot,
            old => {
                if let Some((slot, _)) = old {
                    self.select_pen(NULL_PEN);
                    self.delete(slot);
                }
                let slot = self.alloc();
                self.record(CREATEPENINDIRECT, |o| {
                    o.u16(key.style);
                    o.i16(key.width);
                    o.i16(0);
                    o.u32(key.color);
                });
                self.pen = Some((slot, key));
                slot
            }
        };
        self.select_pen(slot);
    }

    fn fill(&mut self, bp: &BezPath, even_odd: bool, rgb: [u8; 3]) {
        let polys: Vec<Vec<[i16; 2]>> = super::flatten(bp, FLATNESS / self.k).iter().map(|(p, _)| self.points(p)).filter(|p| p.len() > 2).collect();
        if polys.is_empty() {
            return;
        }
        // ALTERNATE (even-odd) or WINDING (non-zero).
        let mode = if even_odd { 1 } else { 2 };
        if self.fill_mode != Some(mode) {
            self.record(SETPOLYFILLMODE, |o| o.u16(mode));
            self.fill_mode = Some(mode);
        }
        self.use_brush(rgb);
        self.select_pen(NULL_PEN);
        for chunk in polys.chunks(usize::from(u16::MAX)) {
            self.record(POLYPOLYGON, |o| {
                o.u16(chunk.len() as u16);
                chunk.iter().for_each(|p| o.u16(p.len() as u16));
                for p in chunk.iter().flatten() {
                    o.i16(p[0]);
                    o.i16(p[1]);
                }
            });
        }
    }

    fn stroke(&mut self, bp: &BezPath, pen: &Pen) {
        let path = super::dashed(bp, pen.dash.as_ref());
        let lines: Vec<(Vec<[i16; 2]>, bool)> =
            super::flatten(&path, FLATNESS / self.k).iter().map(|(p, closed)| (self.points(p), *closed)).filter(|(p, _)| p.len() > 1).collect();
        if lines.is_empty() {
            return;
        }
        self.use_pen(pen);
        self.select_brush(NULL_BRUSH);
        for (pts, closed) in lines {
            // A closed subpath is a polygon (with the null brush, its outline), so it joins at
            // its start.
            self.record(if closed { POLYGON } else { POLYLINE }, |o| {
                o.i16(pts.len() as i16);
                for p in &pts {
                    o.i16(p[0]);
                    o.i16(p[1]);
                }
            });
        }
    }

    /// An image placed by `xf`, upright: false when `xf` rotates or skews it (then it fills its
    /// bounds).
    fn image(&mut self, img: &Image, xf: Affine, opacity: f32) -> bool {
        if img.width == 0 || img.height == 0 || opacity <= 0.0 {
            return true;
        }
        let m = self.to_dev * xf;
        if !m.as_coeffs().iter().all(|v| v.is_finite()) {
            return true;
        }
        let [a, b, c, d, e, f] = m.as_coeffs();
        let (w, h) = (f64::from(img.width), f64::from(img.height));
        let upright = b.abs() < 1e-9 && c.abs() < 1e-9;
        let (x, y, dw, dh) = if upright {
            (e, f, a * w, d * h)
        } else {
            let r = m.transform_rect_bbox(Rect::new(0.0, 0.0, w, h));
            (r.x0, r.y0, r.width(), r.height())
        };
        let s = |v: f64| v.round().clamp(-MAX_I16, MAX_I16) as i16;
        let (bmi, bits) = dib::rgb24(img, opacity);
        self.record(STRETCHDIB, |o| {
            o.u32(SRCCOPY);
            // DIB_RGB_COLORS; the source and destination rectangles, last field first.
            o.u16(0);
            o.i16(img.height as i16);
            o.i16(img.width as i16);
            o.i16(0);
            o.i16(0);
            o.i16(s(dh));
            o.i16(s(dw));
            o.i16(s(y));
            o.i16(s(x));
            o.bytes(&bmi);
            o.bytes(&bits);
        });
        upright
    }
}
