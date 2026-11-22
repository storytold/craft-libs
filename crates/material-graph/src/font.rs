// SPDX-License-Identifier: MIT OR Apache-2.0
//! Fonts for the text node: a small built-in bitmap face and a reader for TrueType outline fonts (`.ttf`).
//!
//! The reader handles the common case: a single TrueType face with a Unicode `cmap` (format 4 or 12) and `glyf`
//! outlines, including composite glyphs. Not supported: OpenType/CFF outlines (`.otf`), font collections, hinting,
//! kerning, complex-script shaping and composite components placed by matching points (only x/y offsets).

use crate::vector::{P, Poly, Seg, Xf, flatten};

pub const GLYPHS: [[u8; 10]; 95] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 4, 4, 4, 4, 4, 0, 4, 0, 0],
    [0, 10, 10, 0, 0, 0, 0, 0, 0, 0],
    [0, 10, 10, 31, 10, 31, 10, 10, 0, 0],
    [0, 4, 15, 20, 14, 5, 30, 4, 0, 0],
    [0, 25, 25, 2, 4, 8, 19, 19, 0, 0],
    [0, 12, 18, 20, 8, 21, 18, 13, 0, 0],
    [0, 4, 4, 0, 0, 0, 0, 0, 0, 0],
    [0, 2, 4, 8, 8, 8, 4, 2, 0, 0],
    [0, 8, 4, 2, 2, 2, 4, 8, 0, 0],
    [0, 0, 21, 14, 31, 14, 21, 0, 0, 0],
    [0, 0, 4, 4, 31, 4, 4, 0, 0, 0],
    [0, 0, 0, 0, 0, 12, 4, 8, 0, 0],
    [0, 0, 0, 0, 31, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 12, 12, 0, 0],
    [0, 1, 1, 2, 4, 8, 16, 16, 0, 0],
    [0, 14, 17, 19, 21, 25, 17, 14, 0, 0],
    [0, 4, 12, 4, 4, 4, 4, 14, 0, 0],
    [0, 14, 17, 1, 2, 4, 8, 31, 0, 0],
    [0, 30, 1, 1, 14, 1, 1, 30, 0, 0],
    [0, 2, 6, 10, 18, 31, 2, 2, 0, 0],
    [0, 31, 16, 30, 1, 1, 17, 14, 0, 0],
    [0, 14, 16, 16, 30, 17, 17, 14, 0, 0],
    [0, 31, 1, 2, 4, 8, 8, 8, 0, 0],
    [0, 14, 17, 17, 14, 17, 17, 14, 0, 0],
    [0, 14, 17, 17, 15, 1, 1, 14, 0, 0],
    [0, 0, 12, 12, 0, 12, 12, 0, 0, 0],
    [0, 0, 12, 12, 0, 12, 4, 8, 0, 0],
    [0, 2, 4, 8, 16, 8, 4, 2, 0, 0],
    [0, 0, 0, 31, 0, 31, 0, 0, 0, 0],
    [0, 8, 4, 2, 1, 2, 4, 8, 0, 0],
    [0, 14, 17, 1, 2, 4, 0, 4, 0, 0],
    [0, 14, 17, 23, 21, 23, 16, 14, 0, 0],
    [0, 14, 17, 17, 31, 17, 17, 17, 0, 0],
    [0, 30, 17, 17, 30, 17, 17, 30, 0, 0],
    [0, 14, 17, 16, 16, 16, 17, 14, 0, 0],
    [0, 30, 17, 17, 17, 17, 17, 30, 0, 0],
    [0, 31, 16, 16, 30, 16, 16, 31, 0, 0],
    [0, 31, 16, 16, 30, 16, 16, 16, 0, 0],
    [0, 14, 17, 16, 23, 17, 17, 15, 0, 0],
    [0, 17, 17, 17, 31, 17, 17, 17, 0, 0],
    [0, 14, 4, 4, 4, 4, 4, 14, 0, 0],
    [0, 7, 2, 2, 2, 2, 18, 12, 0, 0],
    [0, 17, 18, 20, 24, 20, 18, 17, 0, 0],
    [0, 16, 16, 16, 16, 16, 16, 31, 0, 0],
    [0, 17, 27, 21, 21, 17, 17, 17, 0, 0],
    [0, 17, 25, 21, 19, 17, 17, 17, 0, 0],
    [0, 14, 17, 17, 17, 17, 17, 14, 0, 0],
    [0, 30, 17, 17, 30, 16, 16, 16, 0, 0],
    [0, 14, 17, 17, 17, 21, 18, 13, 0, 0],
    [0, 30, 17, 17, 30, 20, 18, 17, 0, 0],
    [0, 15, 16, 16, 14, 1, 1, 30, 0, 0],
    [0, 31, 4, 4, 4, 4, 4, 4, 0, 0],
    [0, 17, 17, 17, 17, 17, 17, 14, 0, 0],
    [0, 17, 17, 17, 17, 17, 10, 4, 0, 0],
    [0, 17, 17, 17, 21, 21, 27, 17, 0, 0],
    [0, 17, 17, 10, 4, 10, 17, 17, 0, 0],
    [0, 17, 17, 10, 4, 4, 4, 4, 0, 0],
    [0, 31, 1, 2, 4, 8, 16, 31, 0, 0],
    [0, 14, 8, 8, 8, 8, 8, 14, 0, 0],
    [0, 16, 16, 8, 4, 2, 1, 1, 0, 0],
    [0, 14, 2, 2, 2, 2, 2, 14, 0, 0],
    [0, 4, 10, 17, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 31, 0, 0],
    [0, 8, 4, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 14, 1, 15, 17, 15, 0, 0],
    [0, 16, 16, 30, 17, 17, 17, 30, 0, 0],
    [0, 0, 0, 14, 17, 16, 17, 14, 0, 0],
    [0, 1, 1, 15, 17, 17, 17, 15, 0, 0],
    [0, 0, 0, 14, 17, 31, 16, 14, 0, 0],
    [0, 6, 9, 8, 28, 8, 8, 8, 0, 0],
    [0, 0, 0, 15, 17, 17, 15, 1, 17, 14],
    [0, 16, 16, 30, 17, 17, 17, 17, 0, 0],
    [0, 4, 0, 12, 4, 4, 4, 14, 0, 0],
    [0, 2, 0, 6, 2, 2, 2, 2, 18, 12],
    [0, 16, 16, 18, 20, 24, 20, 18, 0, 0],
    [0, 12, 4, 4, 4, 4, 4, 14, 0, 0],
    [0, 0, 0, 26, 21, 21, 21, 21, 0, 0],
    [0, 0, 0, 30, 17, 17, 17, 17, 0, 0],
    [0, 0, 0, 14, 17, 17, 17, 14, 0, 0],
    [0, 0, 0, 30, 17, 17, 30, 16, 16, 16],
    [0, 0, 0, 15, 17, 17, 15, 1, 1, 1],
    [0, 0, 0, 22, 25, 16, 16, 16, 0, 0],
    [0, 0, 0, 15, 16, 14, 1, 30, 0, 0],
    [0, 8, 8, 30, 8, 8, 9, 6, 0, 0],
    [0, 0, 0, 17, 17, 17, 19, 13, 0, 0],
    [0, 0, 0, 17, 17, 17, 10, 4, 0, 0],
    [0, 0, 0, 17, 17, 21, 21, 10, 0, 0],
    [0, 0, 0, 17, 10, 4, 10, 17, 0, 0],
    [0, 0, 0, 17, 17, 17, 15, 1, 17, 14],
    [0, 0, 0, 31, 2, 4, 8, 31, 0, 0],
    [0, 6, 8, 8, 16, 8, 8, 6, 0, 0],
    [0, 4, 4, 4, 4, 4, 4, 4, 0, 0],
    [0, 12, 2, 2, 1, 2, 2, 12, 0, 0],
    [0, 0, 0, 13, 22, 0, 0, 0, 0, 0],
];

pub enum Font {
    Builtin,
    TrueType(Box<Ttf>),
}

pub struct Ttf {
    data: Vec<u8>,
    units: f32,
    loca: usize,
    loca_long: bool,
    glyf: usize,
    glyf_len: usize,
    hmtx: usize,
    n_hmtx: usize,
    n_glyphs: usize,
    cmap: Vec<(u32, u32, u32, i32)>, // start, end, first glyph or 0 for delta mapping, delta
    cmap4: Option<(usize, usize)>,
    ascent: f32,
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    let b = d.get(o..o.checked_add(2)?)?;
    Some(u16::from_be_bytes(b.try_into().ok()?))
}
fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    let b = d.get(o..o.checked_add(4)?)?;
    Some(u32::from_be_bytes(b.try_into().ok()?))
}

impl Ttf {
    pub fn parse(data: &[u8]) -> Result<Ttf, String> {
        let tag = data.get(0..4).ok_or("font file is too short")?;
        if tag == b"OTTO" {
            return Err("OpenType fonts with CFF outlines are not supported; use a TrueType (.ttf) font".into());
        }
        if tag == b"ttcf" {
            return Err("font collections (.ttc) are not supported".into());
        }
        if tag != [0, 1, 0, 0] && tag != b"true" {
            return Err("not a TrueType font".into());
        }
        let n = u16_at(data, 4).ok_or("bad font header")? as usize;
        let table = |name: &[u8; 4]| -> Option<(usize, usize)> {
            (0..n).find_map(|i| {
                let r = 12 + i * 16;
                if data.get(r..r + 4)? == name { Some((u32_at(data, r + 8)? as usize, u32_at(data, r + 12)? as usize)) } else { None }
            })
        };
        let (head, _) = table(b"head").ok_or("font has no head table")?;
        let (maxp, _) = table(b"maxp").ok_or("font has no maxp table")?;
        let (hhea, _) = table(b"hhea").ok_or("font has no hhea table")?;
        let (hmtx, _) = table(b"hmtx").ok_or("font has no hmtx table")?;
        let (loca, _) = table(b"loca").ok_or("font has no loca table (not a TrueType outline font)")?;
        let (glyf, glyf_len) = table(b"glyf").ok_or("font has no glyf table (not a TrueType outline font)")?;
        let (cmap, cmap_len) = table(b"cmap").ok_or("font has no cmap table")?;
        let units = u16_at(data, head + 18).ok_or("bad head table")? as f32;
        if units < 16.0 {
            return Err("bad units-per-em".into());
        }
        let loca_long = i16_at(data, head + 50).ok_or("bad head table")? != 0;
        let n_glyphs = u16_at(data, maxp + 4).ok_or("bad maxp table")? as usize;
        let n_hmtx = u16_at(data, hhea + 34).ok_or("bad hhea table")? as usize;
        let ascent = i16_at(data, hhea + 4).ok_or("bad hhea table")? as f32;
        // pick a Unicode subtable
        let count = u16_at(data, cmap + 2).ok_or("bad cmap")? as usize;
        let mut best: Option<(usize, u16)> = None;
        for i in 0..count {
            let r = cmap + 4 + i * 8;
            let (pid, eid, off) =
                (u16_at(data, r).ok_or("bad cmap")?, u16_at(data, r + 2).ok_or("bad cmap")?, u32_at(data, r + 4).ok_or("bad cmap")? as usize);
            let fmt = u16_at(data, cmap + off).unwrap_or(0);
            let unicode = pid == 0 || (pid == 3 && (eid == 1 || eid == 10));
            if unicode && (fmt == 4 || fmt == 12) && best.map(|b| fmt == 12 && b.1 != 12).unwrap_or(true) {
                best = Some((cmap + off, fmt));
            }
        }
        let (sub, fmt) = best.ok_or("font has no Unicode character map this reader understands")?;
        let _ = cmap_len;
        let mut font =
            Ttf { data: data.to_vec(), units, loca, loca_long, glyf, glyf_len, hmtx, n_hmtx, n_glyphs, cmap: Vec::new(), cmap4: None, ascent };
        if fmt == 12 {
            let groups = u32_at(data, sub + 12).ok_or("bad cmap")? as usize;
            for g in 0..groups.min(100_000) {
                let r = sub + 16 + g * 12;
                let (s, e, gl) = (u32_at(data, r).ok_or("bad cmap")?, u32_at(data, r + 4).ok_or("bad cmap")?, u32_at(data, r + 8).ok_or("bad cmap")?);
                font.cmap.push((s, e, gl, 0));
            }
        } else {
            font.cmap4 = Some((sub, 0));
        }
        Ok(font)
    }

    pub fn glyph_index(&self, ch: char) -> usize {
        let c = ch as u32;
        if let Some((sub, _)) = self.cmap4 {
            let d = &self.data;
            let segx2 = u16_at(d, sub + 6).unwrap_or(0) as usize;
            let seg = segx2 / 2;
            let ends = sub + 14;
            let starts = ends + segx2 + 2;
            let deltas = starts + segx2;
            let ranges = deltas + segx2;
            for i in 0..seg {
                let end = u16_at(d, ends + i * 2).unwrap_or(0) as u32;
                if c <= end {
                    let start = u16_at(d, starts + i * 2).unwrap_or(0) as u32;
                    if c < start {
                        return 0;
                    }
                    let delta = i16_at(d, deltas + i * 2).unwrap_or(0) as i32;
                    let ro = u16_at(d, ranges + i * 2).unwrap_or(0) as usize;
                    let g = if ro == 0 {
                        (c as i32 + delta) as u32 & 0xffff
                    } else {
                        let at = ranges + i * 2 + ro + (c - start) as usize * 2;
                        match u16_at(d, at) {
                            Some(0) | None => 0,
                            Some(g) => (g as i32 + delta) as u32 & 0xffff,
                        }
                    };
                    return g as usize;
                }
            }
            return 0;
        }
        for &(s, e, g, _) in &self.cmap {
            if c >= s && c <= e {
                return g.wrapping_add(c - s) as usize;
            }
        }
        0
    }

    pub fn advance(&self, glyph: usize) -> f32 {
        let i = glyph.min(self.n_hmtx.saturating_sub(1));
        u16_at(&self.data, self.hmtx + i * 4).unwrap_or(0) as f32 / self.units
    }

    fn glyph_range(&self, g: usize) -> Option<(usize, usize)> {
        if g >= self.n_glyphs {
            return None;
        }
        let (a, b) = if self.loca_long {
            (u32_at(&self.data, self.loca + g * 4)? as usize, u32_at(&self.data, self.loca + g * 4 + 4)? as usize)
        } else {
            (u16_at(&self.data, self.loca + g * 2)? as usize * 2, u16_at(&self.data, self.loca + g * 2 + 2)? as usize * 2)
        };
        if b <= a || b > self.glyf_len {
            return None;
        }
        Some((self.glyf + a, self.glyf + b))
    }

    /// Outline in em units (y up), as path segments.
    pub fn outline(&self, glyph: usize, depth: u32) -> Vec<Seg> {
        let mut segs = Vec::new();
        let Some((start, end)) = self.glyph_range(glyph) else { return segs };
        // the table directory's glyf length is not checked against the file size
        let d = self.data.get(..end).unwrap_or(&self.data);
        let Some(nc) = i16_at(d, start) else { return segs };
        let u = self.units;
        if nc >= 0 {
            let nc = nc as usize;
            let mut o = start + 10;
            let mut ends = Vec::with_capacity(nc);
            for _ in 0..nc {
                let Some(e) = u16_at(d, o) else { return segs };
                ends.push(e as usize);
                o += 2;
            }
            let npts = ends.last().map(|e| e + 1).unwrap_or(0);
            let Some(il) = u16_at(d, o) else { return segs };
            o += 2 + il as usize;
            let mut flags = Vec::with_capacity(npts);
            while flags.len() < npts {
                let Some(&f) = d.get(o) else { return segs };
                o += 1;
                flags.push(f);
                if f & 8 != 0 {
                    let Some(&r) = d.get(o) else { return segs };
                    o += 1;
                    for _ in 0..r {
                        if flags.len() < npts {
                            flags.push(f);
                        }
                    }
                }
            }
            let mut read = |short: u8, same: u8| -> Option<Vec<i32>> {
                let mut v = Vec::with_capacity(npts);
                let mut cur = 0i32;
                for &f in &flags {
                    if f & short != 0 {
                        let b = *d.get(o)? as i32;
                        o += 1;
                        cur = cur.wrapping_add(if f & same != 0 { b } else { -b });
                    } else if f & same == 0 {
                        cur = cur.wrapping_add(i16_at(d, o)? as i32);
                        o += 2;
                    }
                    v.push(cur);
                }
                Some(v)
            };
            let (Some(xs), Some(ys)) = (read(2, 16), read(4, 32)) else { return segs };
            let mut first = 0usize;
            for &e in &ends {
                if e < first || e >= npts {
                    break;
                }
                let (Some(xr), Some(yr), Some(fr)) = (xs.get(first..=e), ys.get(first..=e), flags.get(first..=e)) else { break };
                let pts: Vec<(P, bool)> = xr.iter().zip(yr).zip(fr).map(|((&x, &y), &f)| ([x as f32 / u, y as f32 / u], f & 1 != 0)).collect();
                contour(&pts, &mut segs);
                first = e + 1;
            }
        } else if depth < 4 {
            let mut o = start + 10;
            while let (Some(fl), Some(gi)) = (u16_at(d, o), u16_at(d, o + 2)) {
                o += 4;
                let (dx, dy) = if fl & 1 != 0 {
                    let r = (i16_at(d, o).unwrap_or(0) as f32, i16_at(d, o + 2).unwrap_or(0) as f32);
                    o += 4;
                    r
                } else {
                    let r = (*d.get(o).unwrap_or(&0) as i8 as f32, *d.get(o + 1).unwrap_or(&0) as i8 as f32);
                    o += 2;
                    r
                };
                let f2 = |o: usize| i16_at(d, o).unwrap_or(0) as f32 / 16384.0;
                let (mut a, mut b, mut c, mut dd) = (1.0, 0.0, 0.0, 1.0);
                if fl & 8 != 0 {
                    a = f2(o);
                    dd = a;
                    o += 2;
                } else if fl & 0x40 != 0 {
                    a = f2(o);
                    dd = f2(o + 2);
                    o += 4;
                } else if fl & 0x80 != 0 {
                    a = f2(o);
                    b = f2(o + 2);
                    c = f2(o + 4);
                    dd = f2(o + 6);
                    o += 8;
                }
                // only offsets are supported for placing a component (point matching is rare)
                let (dx, dy) = if fl & 2 != 0 { (dx / u, dy / u) } else { (0.0, 0.0) };
                let xf = Xf([a, b, c, dd, dx, dy]);
                for s in self.outline(gi as usize, depth + 1) {
                    segs.push(match s {
                        Seg::Move(p) => Seg::Move(xf.apply(p)),
                        Seg::Line(p) => Seg::Line(xf.apply(p)),
                        Seg::Quad(c1, p) => Seg::Quad(xf.apply(c1), xf.apply(p)),
                        Seg::Cubic(c1, c2, p) => Seg::Cubic(xf.apply(c1), xf.apply(c2), xf.apply(p)),
                        Seg::Close => Seg::Close,
                    });
                }
                if fl & 0x20 == 0 {
                    break;
                }
            }
        }
        segs
    }
}

fn contour(pts: &[(P, bool)], out: &mut Vec<Seg>) {
    if pts.len() < 2 {
        return;
    }
    let mid = |a: P, b: P| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    // start at an on-curve point, or at the midpoint of two off-curve points
    let n = pts.len();
    let (Some(&(p_first, _)), Some(&(p_last, _))) = (pts.first(), pts.last()) else { return };
    let (start, begin) = match pts.iter().enumerate().find(|(_, p)| p.1) {
        Some((i, p)) => (p.0, i + 1),
        None => (mid(p_first, p_last), 0),
    };
    out.push(Seg::Move(start));
    let mut ctrl: Option<P> = None;
    let count = if pts.iter().any(|p| p.1) { n - 1 } else { n };
    for k in 0..count {
        let Some(&(p, on)) = pts.get((begin + k) % n) else { continue };
        match (ctrl, on) {
            (None, true) => out.push(Seg::Line(p)),
            (None, false) => ctrl = Some(p),
            (Some(c), true) => {
                out.push(Seg::Quad(c, p));
                ctrl = None;
            }
            (Some(c), false) => {
                out.push(Seg::Quad(c, mid(c, p)));
                ctrl = Some(p);
            }
        }
    }
    if let Some(c) = ctrl {
        out.push(Seg::Quad(c, start));
    }
    out.push(Seg::Close);
}

impl Font {
    pub fn load(bytes: &[u8]) -> Result<Font, String> {
        Ok(Font::TrueType(Box::new(Ttf::parse(bytes)?)))
    }

    /// Advance of one character in em units.
    fn advance(&self, ch: char) -> f32 {
        match self {
            Font::Builtin => 0.65,
            Font::TrueType(t) => t.advance(t.glyph_index(ch)),
        }
    }

    /// Line height in em units.
    pub fn line_height(&self) -> f32 {
        match self {
            Font::Builtin => 1.2,
            Font::TrueType(t) => (t.ascent / t.units * 1.45).max(1.0),
        }
    }

    pub fn ascent(&self) -> f32 {
        match self {
            Font::Builtin => 0.9,
            Font::TrueType(t) => t.ascent / t.units,
        }
    }

    pub fn text_width(&self, line: &str, spacing: f32) -> f32 {
        line.chars().map(|c| self.advance(c) + spacing).sum::<f32>() - if line.is_empty() { 0.0 } else { spacing }
    }

    /// Glyph outlines of one character as polygons in em units (y down, baseline at 0), origin at the pen.
    fn glyph_polys(&self, ch: char, pen: f32, xf_after: &Xf) -> Vec<Poly> {
        match self {
            Font::Builtin => {
                let code = ch as u32;
                if !(32..127).contains(&code) {
                    return vec![];
                }
                let Some(rows) = GLYPHS.get((code - 32) as usize) else { return vec![] };
                // 5 x 10 cell: 1 em = 9 rows, cap height is 7 rows, baseline below row 7
                let px = 1.0 / 9.0 * 0.9;
                let mut out = Vec::new();
                for (r, bits) in rows.iter().enumerate() {
                    for c in 0..5 {
                        if bits & (1 << (4 - c)) != 0 {
                            let x0 = pen + c as f32 * px * 1.0;
                            let y0 = (r as f32 - 8.0) * px;
                            out.push(Poly {
                                pts: vec![
                                    xf_after.apply([x0, y0]),
                                    xf_after.apply([x0 + px, y0]),
                                    xf_after.apply([x0 + px, y0 + px]),
                                    xf_after.apply([x0, y0 + px]),
                                ],
                                closed: true,
                            });
                        }
                    }
                }
                out
            }
            Font::TrueType(t) => {
                let segs = t.outline(t.glyph_index(ch), 0);
                // font units are y-up: flip, then move to the pen position
                let xf = Xf([1.0, 0.0, 0.0, -1.0, pen, 0.0]).then(*xf_after);
                flatten(&segs, &xf)
            }
        }
    }

    /// Lay out text (lines split at '\n') as polygons in pixel space.
    /// `size`: em height in pixels; `align`: 0 left, 1 centre, 2 right; the block is centred on (0, 0) and moved by `xf`.
    pub fn layout(&self, text: &str, size: f32, align: u32, spacing: f32, line_spacing: f32, xf: &Xf) -> Vec<Poly> {
        let lines: Vec<&str> = text.split('\n').collect();
        let lh = self.line_height() * line_spacing;
        let block_h = lh * lines.len() as f32;
        let mut out = Vec::new();
        for (li, line) in lines.iter().enumerate() {
            let w = self.text_width(line, spacing);
            let mut pen = match align {
                1 => -w * 0.5,
                2 => -w,
                _ => 0.0,
            };
            if align == 0 {
                pen = -self.text_width(lines.iter().max_by_key(|l| l.chars().count()).unwrap_or(&""), spacing) * 0.5;
            } else if align == 2 {
                pen += self.text_width(lines.iter().max_by_key(|l| l.chars().count()).unwrap_or(&""), spacing) * 0.5;
            }
            let base_y = (li as f32 + 0.5) * lh - block_h * 0.5 + self.ascent() * 0.35;
            for ch in line.chars() {
                let local = Xf::translate(0.0, base_y).then(Xf::scale(size, size)).then(*xf);
                out.extend(self.glyph_polys(ch, pen, &local));
                pen += self.advance(ch) + spacing;
            }
        }
        out
    }
}
