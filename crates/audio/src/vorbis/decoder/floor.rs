//! Floors (Vorbis I §6 and §7): the spectral envelope each channel's residue is multiplied by.

use super::Error;
use super::bits::{BitReader, Eop, ilog};
use super::codebook::Codebook;
use super::tables::INVERSE_DB;

/// Why a floor could not be decoded from a packet.
pub(super) enum Fail {
    /// The packet ended (a nominal event: the channel is silent).
    Eop,
    /// The packet names a codebook that does not exist: it cannot be decoded (§6.2.2).
    Invalid,
}

impl From<Eop> for Fail {
    fn from(_: Eop) -> Self {
        Fail::Eop
    }
}

/// What a decoded floor gives to synthesis.
pub(super) enum FloorData {
    One { y: Vec<i32>, step2: Vec<bool> },
    Zero { amplitude: u32, coefficients: Vec<f32> },
}

pub(super) struct Floor0 {
    order: usize,
    rate: u32,
    bark_map_size: u32,
    amplitude_bits: u32,
    amplitude_offset: u32,
    books: Vec<usize>,
}

pub(super) struct Floor1 {
    partition_class: Vec<usize>,
    class_dimensions: Vec<usize>,
    class_subclasses: Vec<u32>,
    class_masterbook: Vec<usize>,
    subclass_books: Vec<Vec<i32>>,
    multiplier: i32,
    range: i32,
    xlist: Vec<i32>,
    /// `(low, high)` neighbour positions of each X from index 2 on.
    neighbours: Vec<(usize, usize)>,
    /// Indices of `xlist` in ascending X order.
    sorted: Vec<usize>,
}

pub(super) enum Floor {
    Zero(Floor0),
    One(Floor1),
}

fn eop(_: Eop) -> Error {
    Error::Setup("setup header ends inside a floor")
}

impl Floor {
    pub(super) fn read(floor_type: u32, r: &mut BitReader<'_>, books: &[Codebook]) -> Result<Self, Error> {
        match floor_type {
            0 => {
                let order = r.read(8).map_err(eop)? as usize;
                let rate = r.read(16).map_err(eop)?;
                let bark_map_size = r.read(16).map_err(eop)?;
                let amplitude_bits = r.read(6).map_err(eop)?;
                let amplitude_offset = r.read(8).map_err(eop)?;
                let count = r.read(4).map_err(eop)? as usize + 1;
                let mut list = Vec::with_capacity(count);
                for _ in 0..count {
                    let b = r.read(8).map_err(eop)? as usize;
                    match books.get(b) {
                        Some(book) if book.has_vectors() => list.push(b),
                        _ => return Err(Error::Setup("floor 0 names a missing codebook or one without vectors")),
                    }
                }
                if order == 0 || rate == 0 || bark_map_size == 0 || amplitude_bits > 32 {
                    return Err(Error::Setup("floor 0 with a zero order, rate or map size, or more than 32 amplitude bits"));
                }
                Ok(Self::Zero(Floor0 { order, rate, bark_map_size, amplitude_bits, amplitude_offset, books: list }))
            }
            1 => {
                let partitions = r.read(5).map_err(eop)? as usize;
                let mut partition_class = Vec::with_capacity(partitions);
                for _ in 0..partitions {
                    partition_class.push(r.read(4).map_err(eop)? as usize);
                }
                let classes = partition_class.iter().max().map_or(0, |m| m + 1);
                let (mut class_dimensions, mut class_subclasses, mut class_masterbook, mut subclass_books) = (vec![], vec![], vec![], vec![]);
                for _ in 0..classes {
                    class_dimensions.push(r.read(3).map_err(eop)? as usize + 1);
                    let subs = r.read(2).map_err(eop)?;
                    class_subclasses.push(subs);
                    let master = if subs != 0 { r.read(8).map_err(eop)? as usize } else { 0 };
                    if subs != 0 && master >= books.len() {
                        return Err(Error::Setup("floor 1 master book out of range"));
                    }
                    class_masterbook.push(master);
                    let mut list = Vec::new();
                    for _ in 0..(1u32 << subs) {
                        let b = r.read(8).map_err(eop)? as i32 - 1;
                        if b >= books.len() as i32 {
                            return Err(Error::Setup("floor 1 subclass book out of range"));
                        }
                        list.push(b);
                    }
                    subclass_books.push(list);
                }
                let multiplier = r.read(2).map_err(eop)? as i32 + 1;
                let rangebits = r.read(4).map_err(eop)?;
                let mut xlist = vec![0, 1i32 << rangebits];
                for &c in &partition_class {
                    for _ in 0..class_dimensions[c] {
                        xlist.push(r.read(rangebits).map_err(eop)? as i32);
                    }
                }
                if xlist.len() > 65 {
                    return Err(Error::Setup("floor 1 has more than 65 points"));
                }
                let mut seen = xlist.clone();
                seen.sort_unstable();
                if seen.windows(2).any(|w| w[0] == w[1]) {
                    return Err(Error::Setup("floor 1 repeats an X position"));
                }
                // Low and high neighbours (§9.2.4, §9.2.5) of each point, among those before it.
                let mut neighbours = Vec::new();
                for i in 2..xlist.len() {
                    let (mut lo, mut hi) = (0usize, 1usize);
                    for j in 0..i {
                        if xlist[j] < xlist[i] && xlist[j] > xlist[lo] {
                            lo = j;
                        }
                        if xlist[j] > xlist[i] && xlist[j] < xlist[hi] {
                            hi = j;
                        }
                    }
                    neighbours.push((lo, hi));
                }
                let mut sorted: Vec<usize> = (0..xlist.len()).collect();
                sorted.sort_by_key(|&i| xlist[i]);
                let range = [256, 128, 86, 64][multiplier as usize - 1];
                Ok(Self::One(Floor1 {
                    partition_class,
                    class_dimensions,
                    class_subclasses,
                    class_masterbook,
                    subclass_books,
                    multiplier,
                    range,
                    xlist,
                    neighbours,
                    sorted,
                }))
            }
            _ => Err(Error::Setup("unknown floor type")),
        }
    }

    /// Decode one channel's floor from a packet: `None` when the channel is unused this block.
    pub(super) fn decode(&self, r: &mut BitReader<'_>, books: &[Codebook]) -> Result<Option<FloorData>, Fail> {
        match self {
            Self::One(f) => Ok(f.decode(r, books)?),
            Self::Zero(f) => f.decode(r, books),
        }
    }

    /// The floor curve over `n` bins (`n` is half the block size).
    pub(super) fn curve(&self, data: &FloorData, n: usize) -> Vec<f32> {
        match (self, data) {
            (Self::One(f), FloorData::One { y, step2 }) => f.curve(y, step2, n),
            (Self::Zero(f), FloorData::Zero { amplitude, coefficients }) => f.curve(*amplitude, coefficients, n),
            _ => vec![0.0; n],
        }
    }
}

/// `render_point` (§9.2.6).
fn render_point(x0: i32, y0: i32, x1: i32, y1: i32, x: i32) -> i32 {
    let dy = i64::from(y1 - y0);
    let adx = i64::from(x1 - x0);
    let err = dy.abs() * i64::from(x - x0);
    let off = (err / adx) as i32;
    if dy < 0 { y0 - off } else { y0 + off }
}

/// `render_line` (§9.2.7), writing the bins that fall inside `v`.
fn render_line(x0: i32, y0: i32, x1: i32, y1: i32, v: &mut [i32]) {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let base = dy / adx;
    let ady = dy.abs() - base.abs() * adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let (mut y, mut err) = (y0, 0);
    if let Some(slot) = usize::try_from(x0).ok().and_then(|x| v.get_mut(x)) {
        *slot = y;
    }
    for x in x0 + 1..x1 {
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        match usize::try_from(x).ok().and_then(|x| v.get_mut(x)) {
            Some(slot) => *slot = y,
            None => break,
        }
    }
}

impl Floor1 {
    fn decode(&self, r: &mut BitReader<'_>, books: &[Codebook]) -> Result<Option<FloorData>, Eop> {
        if !r.bit()? {
            return Ok(None);
        }
        let bits = ilog(i64::from(self.range) - 1);
        let mut y = vec![0i32; self.xlist.len()];
        y[0] = r.read(bits)? as i32;
        y[1] = r.read(bits)? as i32;
        let mut offset = 2usize;
        for &class in &self.partition_class {
            let cdim = self.class_dimensions[class];
            let cbits = self.class_subclasses[class];
            let csub = (1u32 << cbits) - 1;
            let mut cval = 0u32;
            if cbits > 0 {
                cval = books[self.class_masterbook[class]].scalar(r)?;
            }
            for j in 0..cdim {
                let book = self.subclass_books[class][(cval & csub) as usize];
                cval >>= cbits;
                y[offset + j] = if book >= 0 { books[book as usize].scalar(r)? as i32 } else { 0 };
            }
            offset += cdim;
        }

        // Step 1: amplitude synthesis (§7.2.4).
        let mut step2 = vec![false; self.xlist.len()];
        step2[0] = true;
        step2[1] = true;
        let mut fin = y.clone();
        for i in 2..self.xlist.len() {
            let (lo, hi) = self.neighbours[i - 2];
            let predicted = render_point(self.xlist[lo], fin[lo], self.xlist[hi], fin[hi], self.xlist[i]);
            let val = y[i];
            let highroom = self.range - predicted;
            let lowroom = predicted;
            let room = if highroom < lowroom { highroom * 2 } else { lowroom * 2 };
            if val != 0 {
                step2[lo] = true;
                step2[hi] = true;
                step2[i] = true;
                fin[i] = if val >= room {
                    if highroom > lowroom { val - lowroom + predicted } else { predicted - val + highroom - 1 }
                } else if val % 2 == 1 {
                    predicted - (val + 1) / 2
                } else {
                    predicted + val / 2
                };
            } else {
                fin[i] = predicted;
            }
            // Valid setups never leave the range; a damaged one must not carry out-of-range values on.
            fin[i] = fin[i].clamp(0, self.range - 1);
        }
        Ok(Some(FloorData::One { y: fin, step2 }))
    }

    fn curve(&self, fin: &[i32], step2: &[bool], n: usize) -> Vec<f32> {
        let mut floor = vec![0i32; n];
        let (mut hx, mut hy) = (0i32, 0i32);
        let (mut lx, mut ly) = (0i32, fin[self.sorted[0]] * self.multiplier);
        for &i in &self.sorted[1..] {
            if step2[i] {
                hy = fin[i] * self.multiplier;
                hx = self.xlist[i];
                render_line(lx, ly, hx, hy, &mut floor);
                lx = hx;
                ly = hy;
            }
        }
        if (hx as usize) < n {
            render_line(hx, hy, n as i32, hy, &mut floor);
        }
        floor.iter().map(|&v| INVERSE_DB[v.clamp(0, 255) as usize]).collect()
    }
}

impl Floor0 {
    fn decode(&self, r: &mut BitReader<'_>, books: &[Codebook]) -> Result<Option<FloorData>, Fail> {
        let amplitude = r.read(self.amplitude_bits)?;
        if amplitude == 0 {
            return Ok(None);
        }
        let book = r.read(ilog(self.books.len() as i64))? as usize;
        let Some(&book) = self.books.get(book) else { return Err(Fail::Invalid) };
        let book = &books[book];
        let mut coefficients: Vec<f32> = Vec::with_capacity(self.order + book.dimensions);
        let mut last = 0f32;
        while coefficients.len() < self.order {
            let Some(v) = book.vector(r)? else { return Err(Fail::Invalid) };
            coefficients.extend(v.iter().map(|&x| x + last));
            last = coefficients[coefficients.len() - 1];
        }
        Ok(Some(FloorData::Zero { amplitude, coefficients }))
    }

    fn curve(&self, amplitude: u32, coefficients: &[f32], n: usize) -> Vec<f32> {
        let bark = |x: f64| 13.1 * (0.00074 * x).atan() + 2.24 * (0.000_000_018_5 * x * x).atan() + 0.0001 * x;
        let scale = f64::from(self.bark_map_size) / bark(0.5 * f64::from(self.rate));
        let map: Vec<i64> = (0..n)
            .map(|i| {
                let foobar = (bark(f64::from(self.rate) * i as f64 / (2.0 * n as f64)) * scale).floor() as i64;
                foobar.min(i64::from(self.bark_map_size) - 1)
            })
            .collect();
        let cos_c: Vec<f64> = coefficients.iter().take(self.order).map(|&c| f64::from(c).cos()).collect();
        let amp = f64::from(amplitude);
        let offset = f64::from(self.amplitude_offset);
        let top = (2f64).powi(self.amplitude_bits as i32) - 1.0;
        let mut out = vec![0f32; n];
        let mut i = 0usize;
        while i < n {
            let omega = std::f64::consts::PI * map[i] as f64 / f64::from(self.bark_map_size);
            let cw = omega.cos();
            let (p, q);
            if self.order % 2 == 1 {
                let mut pp = 1.0 - cw * cw;
                for j in 0..(self.order - 1) / 2 {
                    let d = cos_c[2 * j + 1] - cw;
                    pp *= 4.0 * d * d;
                }
                let mut qq = 0.25;
                for j in 0..=(self.order - 1) / 2 {
                    let d = cos_c[2 * j] - cw;
                    qq *= 4.0 * d * d;
                }
                p = pp;
                q = qq;
            } else {
                let mut pp = (1.0 - cw) / 2.0;
                let mut qq = (1.0 + cw) / 2.0;
                for j in 0..self.order / 2 {
                    let d1 = cos_c[2 * j + 1] - cw;
                    pp *= 4.0 * d1 * d1;
                    let d0 = cos_c[2 * j] - cw;
                    qq *= 4.0 * d0 * d0;
                }
                p = pp;
                q = qq;
            }
            let linear = (0.115_129_25 * (amp * offset / (top * (p + q).sqrt()) - offset)).exp() as f32;
            let key = map[i];
            while i < n && map[i] == key {
                out[i] = linear;
                i += 1;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_line_matches_a_bresenham_walk() {
        let mut v = vec![0i32; 16];
        render_line(0, 0, 8, 4, &mut v);
        assert_eq!(&v[..8], &[0, 0, 1, 1, 2, 2, 3, 3]);
        let mut v = vec![0i32; 16];
        render_line(2, 10, 6, 2, &mut v);
        assert_eq!(&v[2..6], &[10, 8, 6, 4]);
        // Lines running past the vector stop at its end.
        let mut v = vec![0i32; 4];
        render_line(0, 0, 100, 100, &mut v);
        assert_eq!(v, vec![0, 1, 2, 3]);
        assert_eq!(render_point(0, 0, 10, 10, 5), 5);
        assert_eq!(render_point(0, 10, 10, 0, 5), 5);
    }
}
