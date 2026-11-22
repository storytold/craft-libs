//! QuickDraw pictures (PICT, versions 1 and 2), played into neutral metafile items by the opcodes
//! and data lengths of Apple's *Imaging With QuickDraw*, appendix A.
//!
//! A PICT file holds 512 bytes of application data before the picture; a picture on its own (as a
//! `PICT` resource holds it) reads too. A unit is a point, and the picture frame defines the drawing
//! bounds; an extended version 2 header's source rectangle is mapped onto the frame.

use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl, Point, Rect, Vec2};

use crate::canvas::{Raw, RawClip, RawTile};
use crate::dib::Rgba;
use crate::geom::Op;
use crate::text::{Font, HAlign, Text, VAlign};
use crate::{Cap, Error, Frame, Join, LineStyle, MAX_BYTES, MAX_ITEMS, MAX_PIXELS, Picture, clip, dib, f};

/// The application data before the picture in a PICT file.
const FILE_HEADER: usize = 512;
/// Most opcodes one picture plays (a small file can't keep the reader busy for minutes).
const MAX_OPS: usize = 1 << 22;
/// Most rectangles one region breaks into.
const MAX_RECTS: usize = 1 << 16;
/// Steps the regions of one picture may take to read (each inversion point and band costs some).
const MAX_REGION_WORK: u64 = 1 << 28;
/// Most bytes the unpacked rows of one image may hold.
const MAX_ROW_BYTES: u64 = 1 << 28;
/// Most bytes the unpacked rows of one picture's images and patterns may hold together.
const MAX_FILE_ROW_BYTES: u64 = 1 << 30;
/// Most bytes a row may hold past what its pixels need.
const ROW_SLACK: usize = 64;
/// Most pixels, and bytes of unpacked rows, a pixel pattern may hold.
const MAX_PATTERN_PIXELS: usize = 1 << 16;
const MAX_PATTERN_ROW_BYTES: usize = 1 << 20;
/// Curves made for ovals and arcs are this close to the true curve, points.
const TOLERANCE: f64 = 0.01;
/// Pen style flags: square and flat ends.
const SQUARE: u32 = 0x100;
const FLAT: u32 = 0x200;

/// Big-endian reads from a slice, never past its end.
struct Be<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Be<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let b = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(b)
    }
    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let b = self.bytes(N)?;
        b.try_into().ok()
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.pos.checked_add(n)?;
        if end <= self.data.len() {
            self.pos = end;
            Some(())
        } else {
            None
        }
    }
    fn u8(&mut self) -> Option<u8> {
        self.array().map(|[b]: [u8; 1]| b)
    }
    fn u16(&mut self) -> Option<u16> {
        self.array().map(u16::from_be_bytes)
    }
    fn i16(&mut self) -> Option<i16> {
        self.array().map(i16::from_be_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_be_bytes)
    }
    fn i32(&mut self) -> Option<i32> {
        self.array().map(i32::from_be_bytes)
    }
    fn point(&mut self) -> Option<Point> {
        let v = self.i16()?;
        let h = self.i16()?;
        Some(Point::new(h.into(), v.into()))
    }
    fn size(&mut self) -> Option<Vec2> {
        self.point().map(|p| Vec2::new(p.x, p.y))
    }
    fn rect(&mut self) -> Option<Rect> {
        let (top, left, bottom, right) = (self.i16()?, self.i16()?, self.i16()?, self.i16()?);
        Some(Rect::new(left.into(), top.into(), right.into(), bottom.into()))
    }
    fn rgb(&mut self) -> Option<[u8; 3]> {
        let [r, g, b] = [self.u16()?, self.u16()?, self.u16()?];
        Some([r, g, b].map(|c| (c >> 8) as u8))
    }
}

/// Where a picture is, its frame and whether it is version 2.
struct Header {
    frame: Rect,
    v2: bool,
    ops: usize,
}

/// The picture whose `picSize` is at `at`, if a version opcode follows its frame.
fn header(bytes: &[u8], at: usize) -> Option<Header> {
    let mut r = Be::new(bytes, at.checked_add(2)?);
    let frame = r.rect()?;
    let (v2, ops) = match r.array::<2>()? {
        [0x11, 0x01] => (false, r.pos),
        [0x00, 0x11] if r.array::<4>()? == [0x02, 0xFF, 0x0C, 0x00] => (true, at.checked_add(10)?),
        _ => return None,
    };
    (frame.width() > 0.0 && frame.height() > 0.0).then_some(Header { frame, v2, ops })
}

/// The picture of a file (after its application data), else the picture `bytes` start with.
fn find(bytes: &[u8]) -> Option<Header> {
    let v2_at_512 = header(bytes, FILE_HEADER).filter(|h| h.v2);
    let v2_at_0 = header(bytes, 0).filter(|h| h.v2);
    v2_at_512.or(v2_at_0).or_else(|| header(bytes, FILE_HEADER)).or_else(|| header(bytes, 0))
}

/// The size in points from the PICT header when valid.
pub(crate) fn header_size(bytes: &[u8]) -> Option<(f32, f32)> {
    let h = find(bytes)?;
    let w = f(h.frame.width())?;
    let h = f(h.frame.height())?;
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// Do `bytes` hold a picture, after a file's application data or on their own?
pub fn is_pict(bytes: &[u8]) -> bool {
    find(bytes).is_some()
}

#[derive(Clone, Debug)]
enum Pat {
    Bits([u8; 8]),
    Rgb([u8; 3]),
    Tile(Arc<RawTile>),
}

enum PaintChoice {
    Solid([u8; 3]),
    Tile(Arc<RawTile>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verb {
    Frame,
    Paint,
    Erase,
    Invert,
    Fill,
}

fn verb(op: u16) -> Option<Verb> {
    match op & 7 {
        0 => Some(Verb::Frame),
        1 => Some(Verb::Paint),
        2 => Some(Verb::Erase),
        3 => Some(Verb::Invert),
        4 => Some(Verb::Fill),
        _ => None,
    }
}

fn old_color(c: u32) -> [u8; 3] {
    match c {
        30 => [255; 3],
        69 => [255, 255, 0],
        137 => [255, 0, 255],
        205 => [255, 0, 0],
        273 => [0, 255, 255],
        341 => [0, 255, 0],
        409 => [0, 0, 255],
        _ => [0; 3],
    }
}

fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let m = |x: u8, y: u8| (f64::from(x) * t + f64::from(y) * (1.0 - t)).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

fn mac_roman(b: u8) -> char {
    const HIGH: [char; 128] = [
        'Ä', 'Å', 'Ç', 'É', 'Ñ', 'Ö', 'Ü', 'á', 'à', 'â', 'ä', 'ã', 'å', 'ç', 'é', 'è', //
        'ê', 'ë', 'í', 'ì', 'î', 'ï', 'ñ', 'ó', 'ò', 'ô', 'ö', 'õ', 'ú', 'ù', 'û', 'ü', //
        '†', '°', '¢', '£', '§', '•', '¶', 'ß', '®', '©', '™', '´', '¨', '≠', 'Æ', 'Ø', //
        '∞', '±', '≤', '≥', '¥', 'µ', '∂', '∑', '∏', 'π', '∫', 'ª', 'º', 'Ω', 'æ', 'ø', //
        '¿', '¡', '¬', '√', 'ƒ', '≈', '∆', '«', '»', '…', '\u{A0}', 'À', 'Ã', 'Õ', 'Œ', 'œ', //
        '–', '—', '“', '”', '‘', '’', '÷', '◊', 'ÿ', 'Ÿ', '⁄', '€', '‹', '›', 'ﬁ', 'ﬂ', //
        '‡', '·', '‚', '„', '‰', 'Â', 'Ê', 'Á', 'Ë', 'È', 'Í', 'Î', 'Ï', 'Ì', 'Ó', 'Ô', //
        '\u{F8FF}', 'Ò', 'Ú', 'Û', 'Ù', 'ı', 'ˆ', '˜', '¯', '˘', '˙', '˚', '¸', '˝', '˛', 'ˇ',
    ];
    match b {
        0x80.. => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or(char::REPLACEMENT_CHARACTER),
        _ => char::from(b),
    }
}

fn font_family(id: u16) -> &'static str {
    match id {
        0 => "Chicago",
        1 | 3 => "Geneva",
        2 => "New York",
        4 => "Monaco",
        20 => "Times",
        21 => "Helvetica",
        22 => "Courier",
        23 => "Symbol",
        _ => "",
    }
}

fn bez_to_ops(bp: &BezPath) -> Vec<Op> {
    let mut ops = Vec::with_capacity(bp.elements().len());
    for el in bp.elements() {
        match el {
            PathEl::MoveTo(p) => ops.push(Op::M((p.x, p.y))),
            PathEl::LineTo(p) => ops.push(Op::L((p.x, p.y))),
            PathEl::QuadTo(p1, p2) => {
                let last = match ops.last() {
                    Some(Op::M(p) | Op::L(p) | Op::C(_, _, p)) => *p,
                    _ => (0.0, 0.0),
                };
                let c1 = (last.0 + 2.0 / 3.0 * (p1.x - last.0), last.1 + 2.0 / 3.0 * (p1.y - last.1));
                let c2 = (p2.x + 2.0 / 3.0 * (p1.x - p2.x), p2.y + 2.0 / 3.0 * (p1.y - p2.y));
                ops.push(Op::C(c1, c2, (p2.x, p2.y)));
            }
            PathEl::CurveTo(p1, p2, p3) => ops.push(Op::C((p1.x, p1.y), (p2.x, p2.y), (p3.x, p3.y))),
            PathEl::ClosePath => ops.push(Op::Z),
        }
    }
    ops
}

fn round_rect(r: Rect, rx: f64, ry: f64) -> BezPath {
    let r = Rect::new(r.x0.min(r.x1), r.y0.min(r.y1), r.x0.max(r.x1), r.y0.max(r.y1));
    let (rx, ry) = ((rx / 2.0).abs().min(r.width() / 2.0), (ry / 2.0).abs().min(r.height() / 2.0));
    if rx <= 0.0 || ry <= 0.0 {
        let mut bp = BezPath::new();
        bp.move_to((r.x0, r.y0));
        bp.line_to((r.x1, r.y0));
        bp.line_to((r.x1, r.y1));
        bp.line_to((r.x0, r.y1));
        bp.close_path();
        return bp;
    }
    let s = ry / rx;
    let squeezed = Rect::new(r.x0 * s, r.y0, r.x1 * s, r.y1);
    let mut bp = BezPath::new();
    let rr = kurbo::RoundedRect::new(squeezed.x0, squeezed.y0, squeezed.x1, squeezed.y1, ry);
    for el in kurbo::Shape::path_elements(&rr, TOLERANCE) {
        bp.push(el);
    }
    bp.apply_affine(Affine::scale_non_uniform(1.0 / s, 1.0));
    bp
}

fn arc(r: Rect, start: i16, sweep: i16, wedge: bool) -> BezPath {
    let c = r.center();
    let radii = Vec2::new(r.width() / 2.0, r.height() / 2.0);
    let mut bp = BezPath::new();
    if sweep == 0 {
        return bp;
    }
    let a0 = (f64::from(start) - 90.0).to_radians();
    let sweep = f64::from(sweep).clamp(-360.0, 360.0).to_radians();
    let arc = kurbo::Arc { center: c, radii, start_angle: a0, sweep_angle: sweep, x_rotation: 0.0 };
    if wedge {
        bp.move_to(c);
        bp.line_to(c + Vec2::new(radii.x * a0.cos(), radii.y * a0.sin()));
        for el in arc.append_iter(TOLERANCE) {
            bp.push(el);
        }
        bp.close_path();
    } else {
        for el in kurbo::Shape::path_elements(&arc, TOLERANCE) {
            bp.push(el);
        }
    }
    bp
}

fn rects_path(rects: &[Rect]) -> BezPath {
    let mut bp = BezPath::new();
    for r in rects {
        bp.move_to((r.x0, r.y0));
        bp.line_to((r.x1, r.y0));
        bp.line_to((r.x1, r.y1));
        bp.line_to((r.x0, r.y1));
        bp.close_path();
    }
    bp
}

fn spend(work: &mut u64, n: usize) -> bool {
    match work.checked_sub(n as u64) {
        Some(left) => {
            *work = left;
            true
        }
        None => false,
    }
}

fn region(r: &mut Be, work: &mut u64) -> Option<(Vec<Rect>, bool)> {
    let size = usize::from(r.u16()?);
    let mut b = Be::new(r.bytes(size.checked_sub(2)?)?, 0);
    let bbox = b.rect()?;
    if size <= 10 {
        let whole = bbox.width() > 0.0 && bbox.height() > 0.0;
        return Some((if whole { vec![bbox] } else { vec![] }, true));
    }
    Some(bands(&mut b, work).map_or((vec![], false), |rects| (rects, true)))
}

fn bands(b: &mut Be, work: &mut u64) -> Option<Vec<Rect>> {
    let mut out: Vec<Rect> = vec![];
    let mut active: Vec<i16> = vec![];
    let mut last_band = 0;
    let mut prev_y = 0.0;
    loop {
        let y = b.i16()?;
        if y == 0x7FFF {
            break;
        }
        let y = f64::from(y);
        let spans: Vec<(f64, f64)> = active.as_chunks::<2>().0.iter().map(|[a, b]| (f64::from(*a), f64::from(*b))).collect();
        if !spans.is_empty() && y > prev_y {
            if !spend(work, spans.len().saturating_add(1)) {
                return None;
            }
            let prev = out.get_mut(last_band..).unwrap_or_default();
            let same = prev.len() == spans.len() && prev.iter().zip(&spans).all(|(r, (x0, x1))| r.x0 == *x0 && r.x1 == *x1 && r.y1 == prev_y);
            if same {
                prev.iter_mut().for_each(|r| r.y1 = y);
            } else {
                last_band = out.len();
                out.extend(spans.iter().map(|&(x0, x1)| Rect::new(x0, prev_y, x1, y)));
                if out.len() > MAX_RECTS {
                    return None;
                }
            }
        }
        prev_y = y;
        loop {
            let x = b.i16()?;
            if x == 0x7FFF {
                break;
            }
            if !spend(work, active.len().saturating_add(1)) {
                return None;
            }
            match active.binary_search(&x) {
                Ok(i) => {
                    active.remove(i);
                }
                Err(i) => active.insert(i, x),
            }
        }
    }
    Some(out)
}

fn skip(op: u16, r: &mut Be) -> Option<()> {
    let n = match op {
        0x00 | 0x17..=0x19 | 0x1C | 0x1E | 0xB0..=0xCF | 0x8000..=0x80FF => 0,
        0x04 => 1,
        0x03 | 0x05 | 0x08 | 0x0D | 0x11 | 0x15 | 0x16 | 0xA0 => 2,
        0x06 | 0x07 | 0x0B | 0x0C | 0x0E | 0x0F => 4,
        0x1A | 0x1B | 0x1D | 0x1F => 6,
        0x02 | 0x09 | 0x0A | 0x10 => 8,
        0x24..=0x27 | 0x2C..=0x2F | 0x92..=0x97 | 0x9C..=0x9F | 0xA2..=0xAF => usize::from(r.u16()?),
        0x0100..=0x7FFF => usize::from(op >> 8).saturating_mul(2),
        _ => usize::try_from(r.u32()?).ok()?,
    };
    r.skip(n)
}

struct PixMap {
    row_bytes: usize,
    bounds: Rect,
    size: u16,
    pack: u16,
    cmps: u16,
    palette: Option<Vec<[u8; 3]>>,
    packed: bool,
}

impl PixMap {
    fn read(r: &mut Be, pixmap: bool, direct: bool) -> Option<Self> {
        let raw = r.u16()?;
        let mut pm = PixMap { row_bytes: usize::from(raw & 0x3FFF), bounds: r.rect()?, size: 1, pack: 0, cmps: 1, palette: None, packed: true };
        if pixmap || raw & 0x8000 != 0 {
            r.skip(2)?;
            pm.pack = r.u16()?;
            r.skip(14)?;
            pm.size = r.u16()?;
            pm.cmps = r.u16()?;
            r.skip(14)?;
            if !direct {
                pm.palette = Some(color_table(r)?);
            }
        }
        Some(pm)
    }

    fn dims(&self) -> Option<(usize, usize)> {
        let (w, h) = (self.bounds.width(), self.bounds.height());
        (w > 0.0 && h > 0.0).then_some((w as usize, h as usize))
    }

    fn packing(&self) -> u16 {
        match (self.size, self.pack) {
            (32, 0) => 4,
            (16, 0) => 3,
            (_, p) => p,
        }
    }

    fn raw_rows(&self) -> bool {
        !self.packed || self.row_bytes < 8 || self.packing() == 1 || (self.size == 32 && self.packing() == 2)
    }

    fn row_len(&self, w: usize) -> usize {
        match self.packing() {
            4 if self.size == 32 && self.row_bytes >= 8 => w.saturating_mul(usize::from(self.cmps)),
            2 if self.size == 32 => (self.row_bytes / 4).saturating_mul(3),
            _ => self.row_bytes,
        }
    }

    fn decodable(&self, w: usize) -> bool {
        let need = match self.size {
            1 | 2 | 4 | 8 => (w.saturating_mul(usize::from(self.size))).div_ceil(8),
            16 if matches!(self.packing(), 0 | 1 | 3) => 2 * w,
            32 if self.packing() == 4 && self.row_bytes >= 8 => {
                if !matches!(self.cmps, 3 | 4) {
                    return false;
                }
                w.saturating_mul(usize::from(self.cmps))
            }
            32 if self.packing() == 2 => 3 * w,
            32 if matches!(self.packing(), 0 | 1) || (self.packing() == 4 && self.row_bytes < 8) => 4 * w,
            _ => return false,
        };
        (need..=need.saturating_add(ROW_SLACK)).contains(&self.row_len(w))
    }

    fn rows(&self, r: &mut Be, w: usize, h: usize, keep: bool) -> Option<(Vec<u8>, bool)> {
        let len = self.row_len(w);
        let unit = if self.packing() == 3 { 2 } else { 1 };
        let mut out = vec![];
        let mut ok = true;
        let mut keep = keep;
        for _ in 0..h {
            if self.raw_rows() {
                let row = r.bytes(len)?;
                if keep {
                    out.extend_from_slice(row);
                }
                continue;
            }
            let n = if self.row_bytes > 250 { usize::from(r.u16()?) } else { usize::from(r.u8()?) };
            let packed = r.bytes(n)?;
            if keep && !unpack(packed, len, unit, &mut out) {
                ok = false;
                keep = false;
                out = vec![];
            }
        }
        Some((out, ok))
    }

    fn pixel(&self, row: &[u8], x: usize, w: usize, fg: [u8; 3], bg: [u8; 3]) -> Option<[u8; 3]> {
        let at = |i: usize| row.get(i).copied();
        match self.size {
            1 | 2 | 4 | 8 => {
                let bits = usize::from(self.size);
                let shift = 8 - bits - x * bits % 8;
                let index = usize::from((at(x * bits / 8)? >> shift) & ((1u16 << bits) - 1) as u8);
                match &self.palette {
                    Some(p) => p.get(index).copied(),
                    None => Some(if index == 0 { bg } else { fg }),
                }
            }
            16 => {
                let v = u16::from_be_bytes([at(2 * x)?, at(2 * x + 1)?]);
                let c = |s: u16| (((v >> s) & 31) * 255 / 31) as u8;
                Some([c(10), c(5), c(0)])
            }
            _ if self.packing() == 4 && self.row_bytes >= 8 => {
                let first = usize::from(self.cmps).checked_sub(3)?;
                Some([at(first * w + x)?, at((first + 1) * w + x)?, at((first + 2) * w + x)?])
            }
            _ if self.packing() == 2 => Some([at(3 * x)?, at(3 * x + 1)?, at(3 * x + 2)?]),
            _ => Some([at(4 * x + 1)?, at(4 * x + 2)?, at(4 * x + 3)?]),
        }
    }

    fn to_rgba(&self, data: &[u8], w: usize, h: usize, fg: [u8; 3], bg: [u8; 3]) -> Option<Rgba> {
        let len = self.row_len(w);
        let mut pixels = Vec::with_capacity(w.saturating_mul(h).saturating_mul(4));
        for y in 0..h {
            let row = data.get(y * len..(y + 1) * len)?;
            for x in 0..w {
                pixels.extend_from_slice(&self.pixel(row, x, w, fg, bg)?);
                pixels.push(255);
            }
        }
        Some(Rgba { w, h, px: pixels })
    }
}

fn color_table(r: &mut Be) -> Option<Vec<[u8; 3]>> {
    r.skip(4)?;
    let flags = r.u16()?;
    let n = usize::from(r.u16()?).saturating_add(1);
    let mut palette = vec![[0u8; 3]; 256];
    for i in 0..n {
        let value = usize::from(r.u16()?);
        let c = r.rgb()?;
        if let Some(e) = palette.get_mut(if flags & 0x8000 != 0 { i } else { value }) {
            *e = c;
        }
    }
    Some(palette)
}

fn unpack(data: &[u8], len: usize, unit: usize, out: &mut Vec<u8>) -> bool {
    let start = out.len();
    let mut i = 0;
    while out.len().saturating_sub(start) < len {
        let Some(&n) = data.get(i) else {
            return false;
        };
        i += 1;
        let n = n as i8;
        if n >= 0 {
            let k = (n as usize + 1).saturating_mul(unit);
            if out.len().saturating_sub(start).saturating_add(k) > len {
                return false;
            }
            let Some(lit) = data.get(i..i + k) else {
                return false;
            };
            out.extend_from_slice(lit);
            i += k;
        } else if n != -128 {
            let count = (1 - isize::from(n)) as usize;
            let total = count.saturating_mul(unit);
            if out.len().saturating_sub(start).saturating_add(total) > len {
                return false;
            }
            let Some(v) = data.get(i..i + unit) else {
                return false;
            };
            for _ in 0..count {
                out.extend_from_slice(v);
            }
            i += unit;
        }
    }
    out.len().saturating_sub(start) == len
}

fn crop(img: Rgba, src: Rect) -> Option<(Rgba, Rect)> {
    let (w, h) = (img.w, img.h);
    let s = Rect::new(src.x0.min(src.x1), src.y0.min(src.y1), src.x0.max(src.x1), src.y0.max(src.y1));
    let clamp = |v: f64, max: usize| if v.is_finite() { v.round().clamp(0.0, max as f64) as usize } else { 0 };
    let (x0, y0, x1, y1) = (clamp(s.x0, w), clamp(s.y0, h), clamp(s.x1, w), clamp(s.y1, h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let in_bounds = Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64);
    if (x0, y0, x1, y1) == (0, 0, w, h) {
        return Some((img, in_bounds));
    }
    let mut px = Vec::with_capacity((x1 - x0).saturating_mul(y1 - y0).saturating_mul(4));
    for y in y0..y1 {
        let row = y * w * 4;
        if let Some(p) = img.px.get(row + x0 * 4..row + x1 * 4) {
            px.extend_from_slice(p);
        }
    }
    Some((Rgba { w: x1 - x0, h: y1 - y0, px }, in_bounds))
}

struct Pict {
    items: Vec<Raw>,
    frame: Rect,
    base: Affine,
    origin: Vec2,
    fg: [u8; 3],
    bg: [u8; 3],
    pen: Vec2,
    pen_mode: u16,
    pen_pat: Pat,
    fill_pat: Pat,
    back_pat: Pat,
    oval: Vec2,
    pen_loc: Point,
    font: u16,
    face: u8,
    text_size: i16,
    text_loc: Point,
    names: HashMap<u16, String>,
    last: [Rect; 4],
    region_work: u64,
    row_bytes_left: u64,
    pixels_left: u64,
    clip: Option<Vec<[f64; 4]>>,
    pending: BezPath,
    pending_pen: Option<([u8; 3], f64)>,
    skipped_records: u32,
    truncated: bool,
    dropped_images: u32,
    ignored_clips: u32,
    unplayed_raster_ops: u32,
    used_bytes: usize,
}

impl Pict {
    fn new(frame: Rect) -> Self {
        let base = Affine::translate(-frame.origin().to_vec2());
        Self {
            items: Vec::new(),
            frame,
            base,
            origin: Vec2::ZERO,
            fg: [0; 3],
            bg: [255; 3],
            pen: Vec2::new(1.0, 1.0),
            pen_mode: 8,
            pen_pat: Pat::Bits([0xFF; 8]),
            fill_pat: Pat::Bits([0xFF; 8]),
            back_pat: Pat::Bits([0; 8]),
            oval: Vec2::ZERO,
            pen_loc: Point::ZERO,
            font: 0,
            face: 0,
            text_size: 0,
            text_loc: Point::ZERO,
            names: HashMap::new(),
            last: [Rect::ZERO; 4],
            region_work: MAX_REGION_WORK,
            row_bytes_left: MAX_FILE_ROW_BYTES,
            pixels_left: MAX_PIXELS as u64,
            clip: None,
            pending: BezPath::new(),
            pending_pen: None,
            skipped_records: 0,
            truncated: false,
            dropped_images: 0,
            ignored_clips: 0,
            unplayed_raster_ops: 0,
            used_bytes: 0,
        }
    }

    fn xf(&self) -> Affine {
        self.base * Affine::translate(-self.origin)
    }

    fn push(&mut self, raw: Raw) {
        self.push_raw(raw, false);
    }

    fn push_raw(&mut self, mut raw: Raw, keep_region: bool) {
        if self.items.len() >= MAX_ITEMS {
            if matches!(raw, Raw::Bitmap { .. }) {
                self.dropped_images = self.dropped_images.saturating_add(1);
            }
            self.truncated = true;
            return;
        }
        if !keep_region {
            if self.clip.as_ref().is_some_and(Vec::is_empty) {
                return;
            }
            raw.set_region(RawClip { rects: self.clip.clone(), paths: Vec::new() });
        }
        let region_cost = match &raw {
            Raw::Path { region, .. } | Raw::Pattern { region, .. } | Raw::Bitmap { region, .. } | Raw::Text { region, .. } => {
                region.rects.as_ref().map_or(0, |c| c.len().saturating_mul(std::mem::size_of::<[f64; 4]>()))
            }
        };
        let cost = match &raw {
            Raw::Path { ops, .. } => ops.capacity().saturating_mul(std::mem::size_of::<Op>()).saturating_add(64),
            Raw::Pattern { ops, .. } => ops.capacity().saturating_mul(std::mem::size_of::<Op>()).saturating_add(64),
            Raw::Bitmap { img, .. } => img.px.capacity(),
            Raw::Text { text: t, .. } => {
                t.text.capacity().saturating_add(t.dx.capacity().saturating_mul(4)).saturating_add(t.font.family.capacity()).saturating_add(128)
            }
        }
        .saturating_add(region_cost);
        if self.used_bytes.saturating_add(cost) > MAX_BYTES {
            if matches!(raw, Raw::Bitmap { .. }) {
                self.dropped_images = self.dropped_images.saturating_add(1);
            }
            self.truncated = true;
            return;
        }
        self.used_bytes += cost;
        self.items.push(raw);
    }

    fn charge(&mut self, pm: &PixMap, w: usize, h: usize) -> bool {
        let bytes = (pm.row_len(w) as u64).saturating_mul(h as u64);
        let Some(rows_left) = self.row_bytes_left.checked_sub(bytes) else { return false };
        let (Ok(wide), Ok(high)) = (u32::try_from(w), u32::try_from(h)) else { return false };
        let pixels = (wide as u64).saturating_mul(high as u64);
        let Some(px_left) = self.pixels_left.checked_sub(pixels) else { return false };
        self.row_bytes_left = rows_left;
        self.pixels_left = px_left;
        true
    }

    fn op(&mut self, op: u16, r: &mut Be) -> Option<()> {
        if !matches!(op, 0x20..=0x23) {
            self.flush();
        }
        match op {
            0x01 => self.clip(r),
            0x0C => self.move_origin(r),
            0x20..=0x23 => self.line(op, r),
            0x28..=0x2B => self.text(op, r),
            0x2C => self.font_name(r),
            0x30..=0x6F => self.shape(op, r),
            0x70..=0x77 => self.poly(verb(op), r),
            0x80..=0x87 => self.region_shape(verb(op), r),
            0x78..=0x7F | 0x88..=0x8F => Some(()),
            0x90 | 0x91 | 0x98..=0x9B => self.bits(op, r),
            0xA1 => {
                r.skip(2)?;
                let n = usize::from(r.u16()?);
                r.skip(n)
            }
            0x0C00 => self.header(r),
            0x8200 => self.quicktime(r),
            _ => self.state(op, r),
        }
    }

    fn state(&mut self, op: u16, r: &mut Be) -> Option<()> {
        match op {
            0x00 | 0x11 | 0xA0 => return skip(op, r),
            0x02 => self.back_pat = Pat::Bits(r.array()?),
            0x03 => self.font = r.u16()?,
            0x04 => self.face = r.u8()?,
            0x07 => self.pen = r.size()?,
            0x08 => self.pen_mode = r.u16()?,
            0x09 => self.pen_pat = Pat::Bits(r.array()?),
            0x0A => self.fill_pat = Pat::Bits(r.array()?),
            0x0B => self.oval = r.size()?,
            0x0D => self.text_size = r.i16()?,
            0x0E => self.fg = old_color(r.u32()?),
            0x0F => self.bg = old_color(r.u32()?),
            0x12 => self.back_pat = self.pix_pat(r)?,
            0x13 => self.pen_pat = self.pix_pat(r)?,
            0x14 => self.fill_pat = self.pix_pat(r)?,
            0x1A => self.fg = r.rgb()?,
            0x1B => self.bg = r.rgb()?,
            _ => {
                self.skipped_records = self.skipped_records.saturating_add(1);
                return skip(op, r);
            }
        }
        Some(())
    }

    fn header(&mut self, r: &mut Be) -> Option<()> {
        let mut b = Be::new(r.bytes(24)?, 0);
        if b.i16() == Some(-2)
            && let Some(src) = b.skip(10).and_then(|()| b.rect())
            && src.width() > 0.0
            && src.height() > 0.0
        {
            let k = Vec2::new(self.frame.width() / src.width(), self.frame.height() / src.height());
            self.base = Affine::scale_non_uniform(k.x, k.y) * Affine::translate(-src.origin().to_vec2());
        }
        Some(())
    }

    fn move_origin(&mut self, r: &mut Be) -> Option<()> {
        let (dh, dv) = (r.i16()?, r.i16()?);
        self.origin += Vec2::new(dh.into(), dv.into());
        Some(())
    }

    fn pix_pat(&mut self, r: &mut Be) -> Option<Pat> {
        let kind = r.u16()?;
        let bits: [u8; 8] = r.array()?;
        match kind {
            1 => {
                let pm = PixMap::read(r, true, false)?;
                let (w, h) = pm.dims()?;
                let small = w * h <= MAX_PATTERN_PIXELS && pm.row_len(w) * h <= MAX_PATTERN_ROW_BYTES;
                let keep = small && pm.decodable(w) && self.charge(&pm, w, h);
                let (data, _) = pm.rows(r, w, h, keep)?;
                if let Some(img) = keep.then(|| pm.to_rgba(&data, w, h, self.fg, self.bg)).flatten() {
                    let px = img.px.as_chunks::<4>().0;
                    if px.windows(2).any(|w| w[0] != w[1]) {
                        Some(Pat::Tile(Arc::new(RawTile { w: img.w, h: img.h, px: img.px, cell: 1.0 })))
                    } else if let Some(first) = px.first() {
                        Some(Pat::Rgb([first[0], first[1], first[2]]))
                    } else {
                        Some(Pat::Bits(bits))
                    }
                } else {
                    Some(Pat::Bits(bits))
                }
            }
            2 => Some(Pat::Rgb(r.rgb()?)),
            _ => Some(Pat::Bits(bits)),
        }
    }

    fn pat_paint(&self, pat: &Pat) -> PaintChoice {
        match pat {
            Pat::Rgb(c) => PaintChoice::Solid(*c),
            Pat::Tile(tile) => PaintChoice::Tile(tile.clone()),
            Pat::Bits(b) => {
                if *b == [0; 8] {
                    PaintChoice::Solid(self.bg)
                } else if *b == [0xFF; 8] {
                    PaintChoice::Solid(self.fg)
                } else {
                    let mut px = Vec::with_capacity(64 * 4);
                    for row in b {
                        for bit in 0..8 {
                            let is_fg = (row & (0x80 >> bit)) != 0;
                            let c = if is_fg { self.fg } else { self.bg };
                            px.extend([c[0], c[1], c[2], 255]);
                        }
                    }
                    PaintChoice::Tile(Arc::new(RawTile { w: 8, h: 8, px, cell: 1.0 }))
                }
            }
        }
    }

    fn pat_color(&self, pat: &Pat) -> [u8; 3] {
        match pat {
            Pat::Rgb(c) => *c,
            Pat::Tile(tile) => {
                let px = tile.px.as_chunks::<4>().0;
                let n = px.len().max(1) as u64;
                let mut sum = [0u64; 3];
                for p in px {
                    sum[0] += u64::from(p[0]);
                    sum[1] += u64::from(p[1]);
                    sum[2] += u64::from(p[2]);
                }
                [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
            }
            Pat::Bits(b) => {
                let ones: u32 = b.iter().map(|x| x.count_ones()).sum();
                mix(self.fg, self.bg, f64::from(ones) / 64.0)
            }
        }
    }

    fn pen_color(&mut self) -> [u8; 3] {
        if !matches!(self.pen_mode, 0 | 8) {
            self.unplayed_raster_ops = self.unplayed_raster_ops.saturating_add(1);
        }
        self.pat_color(&self.pen_pat)
    }

    fn area_paint(&mut self, verb: Verb) -> Option<PaintChoice> {
        let pat = match verb {
            Verb::Paint => {
                if !matches!(self.pen_mode, 0 | 8) {
                    self.unplayed_raster_ops = self.unplayed_raster_ops.saturating_add(1);
                }
                self.pen_pat.clone()
            }
            Verb::Erase => self.back_pat.clone(),
            Verb::Fill => self.fill_pat.clone(),
            Verb::Frame | Verb::Invert => return None,
        };
        Some(self.pat_paint(&pat))
    }

    fn has_pen(&self) -> bool {
        self.pen.x > 0.0 && self.pen.y > 0.0
    }

    fn fill_paint(&mut self, mut bp: BezPath, paint: PaintChoice, even_odd: bool) {
        bp.apply_affine(self.xf());
        let ops = bez_to_ops(&bp);
        if ops.len() < 2 {
            return;
        }
        let region = RawClip { rects: self.clip.clone(), paths: Vec::new() };
        match paint {
            PaintChoice::Solid(color) => {
                self.push(Raw::Path {
                    ops,
                    fill: Some([color[0], color[1], color[2], 255]),
                    stroke: None,
                    line: LineStyle::default(),
                    even_odd,
                    region,
                });
            }
            PaintChoice::Tile(tile) => {
                self.push(Raw::Pattern { ops, even_odd, tile, region });
            }
        }
    }

    fn push_stroke(&mut self, bp: BezPath, color: [u8; 3], width: f64, cap: Cap) {
        let ops = bez_to_ops(&bp);
        if ops.len() < 2 {
            return;
        }
        self.push(Raw::Path {
            ops,
            fill: None,
            stroke: Some(([color[0], color[1], color[2], 255], width)),
            line: LineStyle { cap, join: Join::Miter, miter_limit: 10.0, dash: Vec::new() },
            even_odd: false,
            region: RawClip { rects: self.clip.clone(), paths: Vec::new() },
        });
    }

    fn fill(&mut self, bp: BezPath, color: [u8; 3], even_odd: bool) {
        self.fill_paint(bp, PaintChoice::Solid(color), even_odd);
    }

    fn scale_factor(&self) -> f64 {
        let c = self.xf().as_coeffs();
        (c[0].abs() + c[3].abs()) / 2.0
    }

    fn stroke(&mut self, mut bp: BezPath, color: [u8; 3], ends: u32) {
        let width = (self.pen.x + self.pen.y) / 2.0 * self.scale_factor();
        let cap = match ends {
            FLAT => Cap::Flat,
            SQUARE => Cap::Square,
            _ => Cap::Round,
        };
        bp.apply_affine(self.xf());
        self.push_stroke(bp, color, width, cap);
    }

    fn frame(&mut self, r: Rect, outline: impl Fn(Rect, Vec2) -> BezPath, ends: u32) {
        if !self.has_pen() {
            return;
        }
        let color = self.pen_color();
        let half = self.pen / 2.0;
        let inner = Rect::new(r.x0 + half.x, r.y0 + half.y, r.x1 - half.x, r.y1 - half.y);
        if inner.width() <= 0.0 || inner.height() <= 0.0 {
            return self.fill(outline(r, self.oval), color, false);
        }
        self.stroke(outline(inner, self.oval - self.pen), color, ends);
    }

    fn shape(&mut self, op: u16, r: &mut Be) -> Option<()> {
        let family = op >> 4;
        let slot = usize::from(family.checked_sub(3)?);
        let rect = if op & 8 != 0 { self.last.get(slot).copied()? } else { r.rect()? };
        let angles = if family == 6 { Some((r.i16()?, r.i16()?)) } else { None };
        let Some(verb) = verb(op) else { return Some(()) };
        if let Some(l) = self.last.get_mut(slot) {
            *l = rect;
        }
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return Some(());
        }
        let outline = move |r: Rect, oval: Vec2, wedge: bool| match family {
            3 => {
                let mut bp = BezPath::new();
                bp.move_to((r.x0, r.y0));
                bp.line_to((r.x1, r.y0));
                bp.line_to((r.x1, r.y1));
                bp.line_to((r.x0, r.y1));
                bp.close_path();
                bp
            }
            4 => round_rect(r, oval.x, oval.y),
            5 => kurbo::Shape::to_path(&kurbo::Ellipse::from_rect(r), TOLERANCE),
            _ => angles.map_or_else(BezPath::new, |(start, sweep)| arc(r, start, sweep, wedge)),
        };
        match verb {
            Verb::Frame => self.frame(rect, |r, oval| outline(r, oval, false), if family == 6 { FLAT } else { SQUARE }),
            Verb::Invert => {
                self.skipped_records = self.skipped_records.saturating_add(1);
            }
            _ => {
                if let Some(paint) = self.area_paint(verb) {
                    self.fill_paint(outline(rect, self.oval, true), paint, false);
                }
            }
        }
        Some(())
    }

    fn poly(&mut self, verb: Option<Verb>, r: &mut Be) -> Option<()> {
        let size = usize::from(r.u16()?);
        let mut b = Be::new(r.bytes(size.checked_sub(2)?)?, 8);
        let Some(verb) = verb else { return Some(()) };
        let mut pts = vec![];
        while let Some(p) = b.point() {
            pts.push(p);
        }
        let Some((first, rest)) = pts.split_first() else { return Some(()) };
        let closed = pts.len() > 2 && pts.first() == pts.last();
        match verb {
            Verb::Frame if self.has_pen() => {
                let half = self.pen / 2.0;
                let mut bp = BezPath::new();
                bp.move_to(*first + half);
                for p in rest {
                    bp.line_to(*p + half);
                }
                if closed {
                    bp.close_path();
                }
                let color = self.pen_color();
                self.stroke(bp, color, SQUARE);
            }
            Verb::Frame => {}
            Verb::Invert => {
                self.skipped_records = self.skipped_records.saturating_add(1);
            }
            _ => {
                let mut bp = BezPath::new();
                bp.move_to(*first);
                for p in rest {
                    bp.line_to(*p);
                }
                bp.close_path();
                if let Some(paint) = self.area_paint(verb) {
                    self.fill_paint(bp, paint, true);
                }
            }
        }
        Some(())
    }

    fn read_region(&mut self, r: &mut Be) -> Option<Vec<Rect>> {
        let (rects, whole) = region(r, &mut self.region_work)?;
        if !whole {
            return None;
        }
        Some(rects)
    }

    fn region_shape(&mut self, verb: Option<Verb>, r: &mut Be) -> Option<()> {
        let Some(rects) = self.read_region(r) else {
            self.skipped_records = self.skipped_records.saturating_add(1);
            return Some(());
        };
        let Some(verb) = verb.filter(|_| !rects.is_empty()) else { return Some(()) };
        match verb {
            Verb::Frame if self.has_pen() => {
                let half = self.pen / 2.0;
                let color = self.pen_color();
                if rects.len() == 1 {
                    let r = rects[0];
                    let inner = Rect::new(r.x0 + half.x, r.y0 + half.y, r.x1 - half.x, r.y1 - half.y);
                    if inner.width() <= 0.0 || inner.height() <= 0.0 {
                        self.fill(rects_path(&[r]), color, false);
                    } else {
                        self.stroke(rects_path(&[inner]), color, SQUARE);
                    }
                } else {
                    let outer_rects: Vec<[f64; 4]> = rects.iter().map(|r| [r.x0, r.y0, r.x1, r.y1]).collect();
                    let inner_rects: Vec<[f64; 4]> = rects
                        .iter()
                        .filter_map(|r| {
                            let (x0, y0, x1, y1) = (r.x0 + half.x, r.y0 + half.y, r.x1 - half.x, r.y1 - half.y);
                            (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
                        })
                        .collect();
                    let diff = clip::combine(Some(outer_rects), inner_rects, clip::RGN_DIFF, None).unwrap_or_default();
                    let diff_rects: Vec<Rect> = diff.into_iter().map(|[x0, y0, x1, y1]| Rect::new(x0, y0, x1, y1)).collect();
                    self.fill(rects_path(&diff_rects), color, false);
                }
            }
            Verb::Frame => {}
            Verb::Invert => {
                self.skipped_records = self.skipped_records.saturating_add(1);
            }
            _ => {
                if let Some(paint) = self.area_paint(verb) {
                    self.fill_paint(rects_path(&rects), paint, false);
                }
            }
        }
        Some(())
    }

    fn clip(&mut self, r: &mut Be) -> Option<()> {
        let (mut rects, whole) = region(r, &mut self.region_work)?;
        if !whole {
            self.ignored_clips = self.ignored_clips.saturating_add(1);
            return Some(());
        }
        if rects.is_empty() {
            let corner = Point::new(self.frame.x0, self.frame.y0);
            rects.push(Rect::from_points(corner, corner));
        }
        let mut mapped = Vec::with_capacity(rects.len());
        for r in rects {
            let p0 = self.xf() * Point::new(r.x0, r.y0);
            let p1 = self.xf() * Point::new(r.x1, r.y1);
            let (x0, x1) = (p0.x.min(p1.x), p0.x.max(p1.x));
            let (y0, y1) = (p0.y.min(p1.y), p0.y.max(p1.y));
            if [x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
                mapped.push([x0, y0, x1, y1]);
            }
        }
        if mapped.len() > clip::MAX_CLIP_RECTS {
            self.ignored_clips = self.ignored_clips.saturating_add(1);
        }
        mapped.truncate(clip::MAX_CLIP_RECTS);
        self.clip = Some(mapped);
        Some(())
    }

    fn flush(&mut self) {
        if !self.pending.elements().is_empty() {
            let bp = std::mem::take(&mut self.pending);
            if let Some((color, width)) = self.pending_pen.take() {
                self.push_stroke(bp, color, width, Cap::Square);
            }
        }
    }

    fn line(&mut self, op: u16, r: &mut Be) -> Option<()> {
        let delta = |r: &mut Be| Some(Vec2::new(f64::from(r.u8()? as i8), f64::from(r.u8()? as i8)));
        let (from, to) = match op {
            0x20 => (r.point()?, r.point()?),
            0x21 => (self.pen_loc, r.point()?),
            0x22 => {
                let a = r.point()?;
                (a, a + delta(r)?)
            }
            _ => (self.pen_loc, self.pen_loc + delta(r)?),
        };
        self.pen_loc = to;
        if !self.has_pen() {
            return Some(());
        }
        let color = self.pen_color();
        if from == to {
            self.flush();
            let mut bp = BezPath::new();
            let p0 = from;
            let p1 = from + self.pen;
            bp.move_to((p0.x, p0.y));
            bp.line_to((p1.x, p0.y));
            bp.line_to((p1.x, p1.y));
            bp.line_to((p0.x, p1.y));
            bp.close_path();
            self.fill(bp, color, false);
            return Some(());
        }
        let width = (self.pen.x + self.pen.y) / 2.0 * self.scale_factor();
        let pen = (color, width);
        if self.pending_pen != Some(pen) {
            self.flush();
            self.pending_pen = Some(pen);
        }
        if self.items.len() >= MAX_ITEMS {
            self.truncated = true;
            return Some(());
        }
        let half = self.pen / 2.0;
        let xf = self.xf();
        let (a, b) = (xf * (from + half), xf * (to + half));
        if self.pending.elements().last().and_then(|e| match e {
            PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(*p),
            _ => None,
        }) != Some(a)
        {
            self.pending.move_to(a);
        }
        self.pending.line_to(b);
        Some(())
    }

    fn text(&mut self, op: u16, r: &mut Be) -> Option<()> {
        match op {
            0x28 => self.text_loc = r.point()?,
            0x29 => self.text_loc.x += f64::from(r.u8()?),
            0x2A => self.text_loc.y += f64::from(r.u8()?),
            _ => {
                self.text_loc.x += f64::from(r.u8()?);
                self.text_loc.y += f64::from(r.u8()?);
            }
        }
        let n = usize::from(r.u8()?);
        let s: String = r.bytes(n)?.iter().map(|&b| mac_roman(b)).filter(|c| !c.is_control() && *c != '\0').collect();
        if s.trim().is_empty() {
            return Some(());
        }
        let size = if self.text_size > 0 { self.text_size as f32 } else { 12.0 };
        let face = self.names.get(&self.font).cloned().unwrap_or_else(|| font_family(self.font).to_string());
        let font = Font {
            family: face,
            height: -size,
            width: 0.0,
            weight: if self.face & 1 != 0 { 700 } else { 400 },
            italic: self.face & 2 != 0,
            underline: self.face & 4 != 0,
            strike: false,
            charset: 0,
        };
        let pt = self.xf() * self.text_loc;
        let c = self.xf().as_coeffs();
        let transform = [c[0], c[1], c[2], c[3], pt.x, pt.y];
        self.push(Raw::Text {
            text: Box::new(Text {
                transform,
                text: s,
                dx: Vec::new(),
                font,
                color: [self.fg[0], self.fg[1], self.fg[2], 255],
                background: None,
                h_align: HAlign::Left,
                v_align: VAlign::Baseline,
                clip: None,
                region: None,
                clips: Vec::new(),
            }),
            region: RawClip { rects: self.clip.clone(), paths: Vec::new() },
        });
        Some(())
    }

    fn font_name(&mut self, r: &mut Be) -> Option<()> {
        let n = usize::from(r.u16()?);
        let mut b = Be::new(r.bytes(n)?, 0);
        if let Some(id) = b.u16()
            && let Some(len) = b.u8()
            && let Some(name) = b.bytes(usize::from(len))
        {
            let name: String = name.iter().map(|&c| mac_roman(c)).collect();
            self.names.insert(id, name.trim().to_string());
        }
        Some(())
    }

    fn bits(&mut self, op: u16, r: &mut Be) -> Option<()> {
        let direct = matches!(op, 0x9A | 0x9B);
        if direct {
            r.skip(4)?;
        }
        let mut pm = PixMap::read(r, direct, direct)?;
        pm.packed = op >= 0x98;
        let (src, dst, mode) = (r.rect()?, r.rect()?, r.u16()?);
        let mask = if op & 1 != 0 {
            match self.read_region(r) {
                Some(m) => Some(m),
                None => {
                    self.ignored_clips = self.ignored_clips.saturating_add(1);
                    None
                }
            }
        } else {
            None
        };
        let (w, h) = pm.dims()?;
        let (wide, high) = (w as u64, h as u64);
        let fits = wide.saturating_mul(high) <= MAX_PIXELS as u64 && (pm.row_len(w) as u64).saturating_mul(high) <= MAX_ROW_BYTES;
        let readable = pm.decodable(w);
        let keep = fits && readable && self.charge(&pm, w, h);
        let (data, ok) = pm.rows(r, w, h, keep)?;
        if !keep {
            self.dropped_images = self.dropped_images.saturating_add(1);
            return Some(());
        }
        let Some(mut img) = pm.to_rgba(&data, w, h, self.fg, self.bg).filter(|_| ok) else {
            self.dropped_images = self.dropped_images.saturating_add(1);
            return Some(());
        };
        self.transfer(&mut img, mode, pm.size == 1 && pm.palette.is_none());
        if dst.width() <= 0.0 || dst.height() <= 0.0 || mask.as_ref().is_some_and(Vec::is_empty) {
            return Some(());
        }
        self.image(img, src - pm.bounds.origin().to_vec2(), dst, mask);
        Some(())
    }

    fn image(&mut self, img: Rgba, src: Rect, dest: Rect, mask: Option<Vec<Rect>>) {
        let Some((cropped, in_bounds)) = crop(img, src) else {
            return;
        };
        let (sw, sh) = (src.width(), src.height());
        if sw <= 0.0 || sh <= 0.0 {
            return;
        }
        let kx = dest.width() / sw;
        let ky = dest.height() / sh;
        let sub_dest = Rect::new(
            dest.x0 + (in_bounds.x0 - src.x0) * kx,
            dest.y0 + (in_bounds.y0 - src.y0) * ky,
            dest.x0 + (in_bounds.x1 - src.x0) * kx,
            dest.y0 + (in_bounds.y1 - src.y0) * ky,
        );
        let (w, h) = (cropped.w as f64, cropped.h as f64);
        let xf = Affine::translate(sub_dest.origin().to_vec2()) * Affine::scale_non_uniform(sub_dest.width() / w, sub_dest.height() / h);
        self.image_xf(cropped, xf, mask);
    }

    fn image_xf(&mut self, img: Rgba, xf: Affine, mask: Option<Vec<Rect>>) {
        if img.w == 0 || img.h == 0 {
            return;
        }
        let xf_doc = self.xf() * xf;
        if !xf_doc.as_coeffs().iter().all(|v| v.is_finite()) || xf_doc.determinant().abs() < 1e-12 {
            return;
        }
        let (w, h) = (img.w as f64, img.h as f64);
        let unit_xf = xf_doc * Affine::scale_non_uniform(w, h);
        let a = unit_xf * Point::new(0.0, 0.0);
        let b = unit_xf * Point::new(1.0, 1.0);
        let ux = unit_xf * Point::new(1.0, 0.0);
        let uy = unit_xf * Point::new(0.0, 1.0);
        let xs = [a.x, b.x, ux.x, uy.x];
        let ys = [a.y, b.y, ux.y, uy.y];
        let x0 = xs.iter().copied().fold(f64::INFINITY, f64::min);
        let x1 = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let y0 = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let y1 = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let rect = [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)];

        let (region, keep_region) = if let Some(mask_rects) = mask {
            let mut mapped = Vec::with_capacity(mask_rects.len());
            for r in mask_rects {
                let p0 = self.xf() * Point::new(r.x0, r.y0);
                let p1 = self.xf() * Point::new(r.x1, r.y1);
                if let Some(norm) = clip::normalize([p0.x, p0.y, p1.x, p1.y]) {
                    mapped.push(norm);
                }
            }
            if mapped.len() > clip::MAX_CLIP_RECTS {
                self.ignored_clips = self.ignored_clips.saturating_add(1);
            }
            mapped.truncate(clip::MAX_CLIP_RECTS);
            let combined = match &self.clip {
                Some(cur) => clip::combine(Some(cur.clone()), mapped, clip::RGN_AND, None),
                None => Some(mapped),
            };
            (RawClip { rects: combined, paths: Vec::new() }, true)
        } else {
            (RawClip { rects: self.clip.clone(), paths: Vec::new() }, false)
        };

        self.push_raw(Raw::Bitmap { rect, xf: unit_xf.as_coeffs(), img, region }, keep_region);
    }

    fn transfer(&mut self, img: &mut Rgba, mode: u16, bitmap: bool) {
        let clear = match mode & !64 {
            0 => false,
            1 if bitmap => true,
            36 => true,
            _ => {
                self.unplayed_raster_ops = self.unplayed_raster_ops.saturating_add(1);
                false
            }
        };
        if clear {
            let bg = self.bg;
            for p in img.px.as_chunks_mut::<4>().0.iter_mut() {
                if [p[0], p[1], p[2]] == bg {
                    p[3] = 0;
                }
            }
        }
    }

    fn quicktime(&mut self, r: &mut Be) -> Option<()> {
        let n = usize::try_from(r.u32()?).ok()?;
        let data = r.bytes(n)?;
        match qt_image(data) {
            Some((ctype, xf, src, image)) if &ctype == b"jpeg" => match self.jpeg(image) {
                Some(img) => {
                    if let Some((img, in_bounds)) = crop(img, src) {
                        self.image_xf(img, xf * Affine::translate(in_bounds.origin().to_vec2()), None);
                    }
                }
                None => self.dropped_images = self.dropped_images.saturating_add(1),
            },
            Some(_) | None => self.dropped_images = self.dropped_images.saturating_add(1),
        }
        Some(())
    }

    fn jpeg(&mut self, data: &[u8]) -> Option<Rgba> {
        let img = dib::decode_compressed(data)?;
        let (wide, high) = (u32::try_from(img.w).ok()?, u32::try_from(img.h).ok()?);
        let pixels = (wide as u64).saturating_mul(high as u64);
        let px_left = self.pixels_left.checked_sub(pixels)?;
        self.pixels_left = px_left;
        Some(img)
    }
}

fn qt_image(d: &[u8]) -> Option<([u8; 4], Affine, Rect, &[u8])> {
    let mut b = Be::new(d, 2);
    let mut m = [0.0; 9];
    for v in &mut m {
        *v = f64::from(b.i32()?) / 65536.0;
    }
    let matte = usize::try_from(b.u32()?).ok()?;
    b.skip(10)?;
    let src = b.rect()?;
    b.skip(4)?;
    let mask = usize::try_from(b.u32()?).ok()?;
    b.skip(matte)?;
    b.skip(mask)?;
    let start = b.pos;
    let id_size = usize::try_from(b.u32()?).ok()?;
    let ctype = b.array::<4>()?;
    b.skip(36)?;
    let size = usize::try_from(b.u32()?).ok()?;
    let at = start.checked_add(id_size)?;
    let data = d.get(at..at.checked_add(size)?).or_else(|| d.get(at..))?;
    let xf = Affine::new([m[0], m[1], m[3], m[4], m[6], m[7]]);
    Some((ctype, xf, src, data))
}

/// Read a QuickDraw picture.
pub fn parse(bytes: &[u8]) -> Result<Picture, Error> {
    crate::dib::set_decode_budget(crate::dib::MAX_DECODE_PIXELS);
    parse_nested(bytes)
}

/// [`parse`] for a picture inside the metafile being parsed: it draws on the outer file's decode budget.
pub(crate) fn parse_nested(bytes: &[u8]) -> Result<Picture, Error> {
    let h = find(bytes).ok_or(Error::NotPict)?;
    let mut q = Pict::new(h.frame);
    let mut r = Be::new(bytes, h.ops);
    let mut stopped_early = true;
    for _ in 0..MAX_OPS {
        if h.v2 && r.pos % 2 == 1 && r.skip(1).is_none() {
            break;
        }
        let Some(op) = (if h.v2 { r.u16() } else { r.u8().map(u16::from) }) else {
            break;
        };
        if op == 0x00FF {
            stopped_early = false;
            break;
        }
        if q.op(op, &mut r).is_none() {
            break;
        }
    }
    if stopped_early {
        q.truncated = true;
    }
    q.flush();
    let pic =
        crate::build(q.items, Some(Frame { rect: [0.0, 0.0, h.frame.width(), h.frame.height()], size: Some((h.frame.width(), h.frame.height())) }));
    Ok(Picture {
        skipped_records: q.skipped_records.saturating_add(pic.skipped_records),
        truncated: q.truncated || pic.truncated,
        dropped_images: q.dropped_images.saturating_add(pic.dropped_images),
        ignored_clips: q.ignored_clips.saturating_add(pic.ignored_clips),
        unplayed_raster_ops: q.unplayed_raster_ops.saturating_add(pic.unplayed_raster_ops),
        ..pic
    })
}
