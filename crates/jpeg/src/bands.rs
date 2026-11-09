//! Band-at-a-time decoding of sequential Huffman-coded 8-bit JPEG: see [`Bands`].

use crate::{
    Bits, Error, Header, Huffman, HuffmanScan, Idct, Limits, ScanDecoder, Tables, be16, err, idct_basis, idct_block, is_ycc, parse_dac, parse_dht,
    parse_dqt, parse_header, parse_scan, to_color, to_u8, unsupported, upsample_map, ycc_to_rgb,
};

/// A component as the bands decode it.
struct Comp {
    /// Blocks of the component in one MCU, across and down: H × V in an interleaved scan, 1 × 1
    /// when it is the scan's only component.
    hb: usize,
    vb: usize,
    /// Samples per row of an MCU row's plane: the blocks of a line of MCUs, × 8.
    stride: usize,
    /// The quantization table (zigzag order) as it was at the scan.
    q: [u16; 64],
    /// The interpolation of a component subsampled across or down, as the whole-image decode
    /// makes it ([`upsample_map`]); `None` when it has a sample for every output column or row.
    xs: Option<Vec<(usize, usize, f32)>>,
    ys: Option<Vec<(usize, usize, f32)>>,
}

/// A sequential Huffman-coded (SOF0, SOF1) 8-bit JPEG whose first scan holds every component,
/// decoded a band of rows at a time so that no buffer the size of the image exists.
///
/// A band is the rows of one line of MCUs (A.2): 8 times the largest vertical sampling factor
/// (8 for a single-component frame); the last band may be shorter. A [`Reader`] decodes bands
/// in order, into samples equal to [`decode`](crate::decode)'s for the same rows: the same
/// inverse transform, upsampling and colour conversion, at most a few MCU lines of them at a
/// time.
///
/// The JPEG processes [`decode`](crate::decode) covers beyond this (arithmetic coding,
/// progressive and 12-bit frames) and frames whose components come in several scans return
/// [`Error::Unsupported`]: decode those whole.
///
/// Bands can also be decoded out of order, by [`Reader`]s on separate threads: a restart
/// interval (B.2.4.4, `DRI`) that begins with a band makes the band a seek point once
/// [`Bands::find_seek_points`] has found the restart marker before it.
///
/// **Damaged data.** Entropy-coded data that is cut short or corrupt leaves the rest of its
/// restart interval at zero coefficients (mid grey), and decoding resumes at the next restart
/// marker; without restart intervals the rest of the scan stays at zero, as in
/// [`decode`](crate::decode), which does not resume at restart markers. Either way
/// [`Reader::damaged`] is set, and the result does not depend on where the decoding started.
/// An interval, or the scan, that leaves more than its padding unread before the next marker is
/// damaged too (its data was cut, shifted or added to). Damaged data never makes a read fail,
/// also in the first MCU, where [`decode`](crate::decode) reports that the stream holds no image
/// data. [`Reader::damaged`] does not tell that case from later damage, and neither does the
/// size of the scan: data that fails at its first code can be any length.
pub struct Bands<'a> {
    /// The file from the scan's entropy-coded data on.
    data: &'a [u8],
    width: usize,
    height: usize,
    /// In frame order (the output's channel order).
    comps: Vec<Comp>,
    /// The scan's components in scan order: frame index, DC and AC table.
    scan: Vec<(usize, Huffman, Huffman)>,
    /// MCUs per line and lines of MCUs (bands) of the scan.
    mcux: usize,
    mcuy: usize,
    band_rows: usize,
    /// MCUs per restart interval; 0 without restart intervals.
    restart: usize,
    ycc: bool,
    /// 1 when a component is subsampled down: a band's rows then read the last sample row of the
    /// MCU line above it and the first of the one below.
    lag: usize,
    /// Bands decoding can start at, with the offset of their entropy-coded data in `data`;
    /// ascending, band 0 first.
    seek: Vec<(usize, usize)>,
}

impl<'a> Bands<'a> {
    /// Reads the headers of `bytes` up to the first scan.
    ///
    /// `limits.max_pixels` bounds the image; `limits.max_alloc` bounds what one [`Reader`]
    /// holds ([`Bands::reader_bytes`]) and the sampling maps kept here, not the image.
    /// `adobe_transform` is the transform byte of the APP14 `Adobe` segment, as for
    /// [`decode`](crate::decode).
    pub fn new(bytes: &'a [u8], limits: &Limits, adobe_transform: Option<u8>) -> Result<Self, Error> {
        if bytes.get(..2) != Some(&[0xFF, 0xD8]) {
            return Err(err("not a JPEG stream"));
        }
        let mut tables = Tables::default();
        let mut header: Option<Header> = None;
        let mut i = 2;
        while let Some(&v) = bytes.get(i) {
            if v != 0xFF {
                i += 1;
                continue;
            }
            let Some(&m) = bytes.get(i + 1) else { break };
            match m {
                0xFF => {
                    i += 1;
                    continue;
                }
                0x00 | 0x01 | 0xD0..=0xD8 => {
                    i += 2;
                    continue;
                }
                0xD9 => break,
                _ => {}
            }
            let Ok(len) = be16(bytes, i + 2) else { break };
            let len = usize::from(len);
            if len < 2 {
                return Err(err("bad marker segment length"));
            }
            let seg = bytes.get(i + 4..i + 2 + len).ok_or_else(|| err("marker segment runs past the end of the file"))?;
            i += 2 + len;
            match m {
                0xDB => parse_dqt(seg, &mut tables)?,
                0xC4 => parse_dht(seg, &mut tables)?,
                0xCC => parse_dac(seg, &mut tables)?,
                0xDD => tables.restart = usize::from(be16(seg, 0)?),
                0xC0..=0xCF => {
                    if header.is_some() {
                        return Err(unsupported("JPEG with more than one frame is not supported"));
                    }
                    let hd = parse_header(m, seg)?;
                    if hd.arithmetic || hd.progressive || hd.precision != 8 {
                        return Err(unsupported("only sequential Huffman-coded 8-bit JPEG is decoded in bands"));
                    }
                    header = Some(hd);
                }
                0xDA => {
                    let hd = header.ok_or_else(|| err("scan before the frame header"))?;
                    let data = bytes.get(i..).unwrap_or_default();
                    return Self::open(data, hd, seg, &tables, limits, adobe_transform);
                }
                _ => {}
            }
        }
        Err(err(if header.is_some() { "the file ends before any image data" } else { "no frame header" }))
    }

    /// The bands of the frame `hd` and its first scan `sos`, with the tables in effect at the
    /// scan; checked against `limits` before any buffer is sized from the headers.
    fn open(data: &'a [u8], hd: Header, sos: &[u8], tables: &Tables, limits: &Limits, adobe_transform: Option<u8>) -> Result<Self, Error> {
        let ids: Vec<u8> = hd.comps.iter().map(|c| c.0).collect();
        let scan = parse_scan(sos, &ids, false)?;
        let mut seen = [false; 4];
        for &(ci, _, _) in &scan.comps {
            match seen.get_mut(ci) {
                Some(s) if !*s => *s = true,
                // T.81 B.2.3: a scan's components follow the frame's order, so none repeats.
                _ => return Err(err("the scan names a component twice")),
            }
        }
        if scan.comps.len() != hd.comps.len() {
            return Err(unsupported("JPEG whose components are in more than one scan is not decoded in bands"));
        }
        let (width, height) = (hd.width, hd.height);
        if width == 0 || height == 0 {
            return Err(err(format!("zero-sized image {width}x{height}")));
        }
        let pixels = (width as u64).saturating_mul(height as u64);
        if pixels > limits.max_pixels {
            return Err(Error::LimitExceeded(format!("{pixels} pixels exceed max {}", limits.max_pixels)));
        }
        let single = scan.comps.len() == 1;
        let (mcux, mcuy) = match hd.comps.first() {
            Some(&(_, h, v, _)) if single => {
                let (cw, ch) = hd.samples(h, v);
                (cw.div_ceil(8), ch.div_ceil(8))
            }
            _ => (hd.mcux, hd.mcuy),
        };
        let band_rows = if single { 8 } else { 8 * hd.vmax };
        let mut comps = Vec::with_capacity(hd.comps.len());
        for &(_, h, v, tq) in &hd.comps {
            let (hb, vb) = if single { (1, 1) } else { (h, v) };
            let q = tables.quant.get(tq).copied().flatten().ok_or_else(|| err("scan uses an undefined quantization table"))?;
            // The interpolation maps are made once the limit check below has passed.
            comps.push(Comp { hb, vb, stride: mcux * hb * 8, q, xs: None, ys: None });
        }
        let mut coding = Vec::with_capacity(scan.comps.len());
        for &(ci, td, ta) in &scan.comps {
            let dc = tables.dc.get(td).and_then(Option::as_ref).ok_or_else(|| err("scan uses an undefined Huffman table"))?;
            let ac = tables.ac.get(ta).and_then(Option::as_ref).ok_or_else(|| err("scan uses an undefined Huffman table"))?;
            coding.push((ci, dc.clone(), ac.clone()));
        }
        let lag = usize::from(hd.comps.iter().any(|&(_, _, v, _)| v != hd.vmax));
        let mut bands = Bands {
            data,
            width,
            height,
            comps,
            scan: coding,
            mcux,
            mcuy,
            band_rows,
            restart: tables.restart,
            ycc: is_ycc(ids.into_iter(), adobe_transform),
            lag,
            seek: vec![(0, 0)],
        };
        // Checked before the maps are made: one entry per output column (or row) of each
        // component subsampled across (or down).
        let entry = std::mem::size_of::<(usize, usize, f32)>() as u64;
        let maps: u64 =
            hd.comps.iter().map(|&(_, h, v, _)| (u64::from(h != hd.hmax) * width as u64 + u64::from(v != hd.vmax) * height as u64) * entry).sum();
        let total = bands.reader_bytes().saturating_add(maps);
        let cap = limits.max_alloc.min(isize::MAX as u64);
        if total > cap {
            return Err(Error::LimitExceeded(format!("{total} bytes of band buffers exceed the limit of {cap}")));
        }
        for (&(_, h, v, _), c) in hd.comps.iter().zip(&mut bands.comps) {
            let (cw, ch) = hd.samples(h, v);
            c.xs = (h != hd.hmax).then(|| upsample_map(width, cw, h, hd.hmax));
            c.ys = (v != hd.vmax).then(|| upsample_map(height, ch, v, hd.vmax));
        }
        Ok(bands)
    }

    /// The image's width in pixels.
    pub fn width(&self) -> u32 {
        self.width as u32
    }

    /// The image's height in pixels: the frame's (a `DNL` height is not supported).
    pub fn height(&self) -> u32 {
        self.height as u32
    }

    /// Samples per pixel, as [`Decoded::components`](crate::Decoded::components) describes them:
    /// 1 gray, 3 RGB, 4 CMYK.
    pub fn components(&self) -> u8 {
        self.comps.len() as u8
    }

    /// Rows in a band (the last one may have fewer).
    pub fn band_rows(&self) -> u32 {
        self.band_rows as u32
    }

    /// How many bands make up the image.
    pub fn bands(&self) -> usize {
        self.mcuy
    }

    /// MCU lines a [`Reader`] decodes on each side of the bands it hands out: 1 when a component
    /// is subsampled down (a band's rows interpolate with the lines above and below), else 0.
    /// When it is 1, a [`Reader`] that [seeks](Reader::seek) to seek point `p` starts decoding at
    /// the seek point `q` before it: reading `n` bands from `p` decodes `p − q + n + 1` lines,
    /// `n + 2` when restart intervals begin every line of MCUs. A caller that splits the work
    /// reads a few bands from each seek point then.
    pub fn halo(&self) -> u32 {
        self.lag as u32
    }

    /// Bytes of the samples of a full band.
    pub fn band_bytes(&self) -> usize {
        self.band_rows * self.width * self.comps.len()
    }

    /// Bytes one [`Reader`] holds while it decodes: the reconstructed MCU lines it keeps, one
    /// interpolated row per component and a band of samples.
    pub fn reader_bytes(&self) -> u64 {
        let slots = (2 * self.lag + 1) as u64;
        let planes: u64 = self.comps.iter().map(|c| slots * (c.vb * 8 * c.stride) as u64 * 4).sum();
        let lines = (self.comps.len() * self.width) as u64 * 4;
        planes.saturating_add(lines).saturating_add(self.band_bytes() as u64)
    }

    /// Finds the restart markers in the scan, so that every band a restart interval begins with
    /// becomes a seek point. One pass over the entropy-coded data. The k-th restart marker in the
    /// data begins interval k, whatever its number: as a reader that reads in order pairs them,
    /// so that reading from a seek point gives the same samples also when markers are missing,
    /// misnumbered or added (damaged data), and the bands past the last marker found are reached
    /// in order from the last seek point.
    pub fn find_seek_points(&mut self) {
        self.seek.truncate(1);
        let (restart, mcux) = (self.restart, self.mcux);
        if restart == 0 {
            return;
        }
        let intervals = self.mcux.saturating_mul(self.mcuy).div_ceil(restart);
        let d = self.data;
        let (mut i, mut k) = (0usize, 0usize);
        while let Some(p) = d.get(i..).and_then(|r| r.iter().position(|&v| v == 0xFF)) {
            i += p;
            match d.get(i + 1) {
                Some(0x00) => i += 2,
                Some(0xFF) => i += 1,
                Some(0xD0..=0xD7) => {
                    k += 1;
                    if k >= intervals {
                        return;
                    }
                    let first = k * restart;
                    if first.is_multiple_of(mcux) {
                        self.seek.push((first / mcux, i + 2));
                    }
                    i += 2;
                }
                _ => return,
            }
        }
    }

    /// The bands a [`Reader`] can [seek](Reader::seek) to, ascending; band 0 always.
    pub fn seek_points(&self) -> impl ExactSizeIterator<Item = usize> + '_ {
        self.seek.iter().map(|s| s.0)
    }

    /// A reader at band 0.
    pub fn reader(&self) -> Reader<'_> {
        let slots = 2 * self.lag + 1;
        Reader {
            dec: HuffmanScan { bits: Bits::new(self.data), tables: self.scan.iter().map(|(_, dc, ac)| (dc, ac)).collect(), pred: [0; 4] },
            planes: self.comps.iter().map(|c| vec![0f32; slots * c.vb * 8 * c.stride]).collect(),
            lines: self.comps.iter().map(|c| if c.xs.is_some() || c.ys.is_some() { vec![0f32; self.width] } else { Vec::new() }).collect(),
            out: vec![0u8; self.band_bytes()],
            idct: idct_basis(),
            bands: self,
            decoded: 0,
            next: 0,
            fresh: true,
            skip_to: 0,
            damaged: false,
        }
    }
}

/// One band of samples, interleaved by pixel, rows top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band<'r> {
    /// Its index: rows from `index × band_rows` on.
    pub index: usize,
    /// The image row of its first row.
    pub y: u32,
    pub rows: u32,
    pub samples: &'r [u8],
}

/// Decodes [`Bands`] in order from a seek point.
pub struct Reader<'b> {
    bands: &'b Bands<'b>,
    dec: HuffmanScan<'b>,
    /// Per component: the reconstructed samples of the last `2 × lag + 1` MCU lines decoded,
    /// line `m` in slot `m % slots`.
    planes: Vec<Vec<f32>>,
    /// Per subsampled component: one output row of interpolated samples.
    lines: Vec<Vec<f32>>,
    out: Vec<u8>,
    idct: Idct,
    /// The next MCU line to decode and the next band to hand out.
    decoded: usize,
    next: usize,
    /// At a seek point: the first MCU follows its restart marker already.
    fresh: bool,
    /// MCUs before this one are left at zero coefficients (damaged data).
    skip_to: usize,
    damaged: bool,
}

impl<'b> Reader<'b> {
    /// Moves to `band`, which must be one of [`Bands::seek_points`]: the next band handed out is
    /// `band`. When [`Bands::halo`] is 1 the decoding starts at the seek point before `band`, for
    /// the line above it.
    pub fn seek(&mut self, band: usize) -> Result<(), Error> {
        let b = self.bands;
        let at = b.seek.binary_search_by_key(&band, |s| s.0).map_err(|_| unsupported(format!("band {band} is not a seek point")))?;
        // A band reads the last sample row of the MCU line above it when a component is
        // subsampled down: start at the seek point before it.
        let from = if b.lag > 0 && at > 0 { at - 1 } else { at };
        let &(first, offset) = b.seek.get(from).ok_or_else(|| unsupported("no seek point"))?;
        self.dec.bits = Bits { pos: offset, ..Bits::new(b.data) };
        self.dec.pred = [0; 4];
        self.decoded = first;
        self.next = band;
        self.fresh = true;
        self.skip_to = 0;
        Ok(())
    }

    /// Whether any data decoded so far was damaged (see [`Bands`]).
    pub fn damaged(&self) -> bool {
        self.damaged
    }

    /// The next band, or `None` past the last one.
    pub fn next_band(&mut self) -> Result<Option<Band<'_>>, Error> {
        let b = self.bands;
        if self.next >= b.mcuy {
            return Ok(None);
        }
        let need = (self.next + b.lag).min(b.mcuy - 1);
        while self.decoded <= need {
            self.decode_line(self.decoded)?;
            self.decoded += 1;
        }
        let index = self.next;
        let y = index * b.band_rows;
        let rows = b.band_rows.min(b.height - y);
        self.render(y, rows);
        self.next += 1;
        let len = rows * b.width * b.comps.len();
        Ok(Some(Band { index, y: y as u32, rows: rows as u32, samples: self.out.get(..len).unwrap_or_default() }))
    }

    /// Entropy-decodes MCU line `m` and reconstructs its blocks into the planes.
    fn decode_line(&mut self, m: usize) -> Result<(), Error> {
        let b = self.bands;
        let slots = 2 * b.lag + 1;
        let shift = 128.0;
        for mx in 0..b.mcux {
            let i = m * b.mcux + mx;
            if b.restart > 0 && i > 0 && i.is_multiple_of(b.restart) && !self.fresh {
                self.dec.restart();
            }
            self.fresh = false;
            for (sc, &(ci, _, _)) in b.scan.iter().enumerate() {
                let (Some(c), Some(plane)) = (b.comps.get(ci), self.planes.get_mut(ci)) else { return Err(err("bad scan component")) };
                let base = (m % slots) * c.vb * 8 * c.stride;
                for by in 0..c.vb {
                    for bx in 0..c.hb {
                        let at = base + by * 8 * c.stride + (mx * c.hb + bx) * 8;
                        let out = plane.get_mut(at..).unwrap_or_default();
                        if i < self.skip_to {
                            for y in 0..8 {
                                if let Some(row) = out.get_mut(y * c.stride..y * c.stride + 8) {
                                    row.fill(shift);
                                }
                            }
                            continue;
                        }
                        let mut blk = [0i32; 64];
                        if self.dec.block(sc, &mut blk).is_err() {
                            // The rest of the restart interval (or of the scan) stays at zero;
                            // the partly decoded block is kept, as the whole-image decode keeps it.
                            self.damaged = true;
                            self.skip_to = i.checked_div(b.restart).map_or(usize::MAX, |k| (k + 1) * b.restart);
                        }
                        idct_block(&blk, &c.q, &self.idct, 255.0, out, c.stride);
                    }
                }
            }
            // An interval, or the scan, that ends with more than its padding before the next
            // marker decoded from data that was cut, shifted or added to: damaged, though no code
            // was invalid.
            let end = i + 1;
            let total = b.mcux * b.mcuy;
            let interval_end = b.restart > 0 && end.is_multiple_of(b.restart);
            if (interval_end || end == total) && i >= self.skip_to && self.dec.bits.data_left() {
                self.damaged = true;
            }
        }
        Ok(())
    }

    /// Writes output rows `y..y + rows` into `out`.
    fn render(&mut self, y: usize, rows: usize) {
        let Reader { bands: b, planes, lines, out, decoded, .. } = self;
        let (b, decoded) = (*b, *decoded);
        let (w, nc) = (b.width, b.comps.len());
        for r in 0..rows {
            let yy = y + r;
            // Subsampled components: interpolated as the whole-image decode does.
            for ((c, comp), line) in b.comps.iter().enumerate().zip(lines.iter_mut()) {
                if comp.xs.is_none() && comp.ys.is_none() {
                    continue;
                }
                let (y0, y1, fy) = comp.ys.as_ref().and_then(|m| m.get(yy).copied()).unwrap_or((yy, yy, 0.0));
                let (r0, r1) = (plane_row(b, planes, decoded, c, y0), plane_row(b, planes, decoded, c, y1));
                let at = |row: &[f32], x: usize| row.get(x).copied().unwrap_or(0.0);
                for (x, o) in line.iter_mut().enumerate() {
                    let (x0, x1, fx) = comp.xs.as_ref().and_then(|m| m.get(x).copied()).unwrap_or((x, x, 0.0));
                    let top = at(r0, x0) + (at(r0, x1) - at(r0, x0)) * fx;
                    let bottom = at(r1, x0) + (at(r1, x1) - at(r1, x0)) * fx;
                    *o = top + (bottom - top) * fy;
                }
            }
            let mut src: [&[f32]; 4] = [&[]; 4];
            for (((c, comp), line), s) in b.comps.iter().enumerate().zip(lines.iter()).zip(src.iter_mut()) {
                let row = if comp.xs.is_none() && comp.ys.is_none() { plane_row(b, planes, decoded, c, yy) } else { line.as_slice() };
                *s = row.get(..w).unwrap_or_default();
            }
            let Some(out) = out.get_mut(r * w * nc..(r + 1) * w * nc) else { continue };
            convert_row(&src, nc, b.ycc, out);
        }
    }
}

/// One output row from the components' rows `src` (`w` samples each; a short one reads as
/// zeros): the colour conversion and rounding of [`to_color`] and [`to_u8`], with the common
/// layouts written out so they compile to straight loops.
fn convert_row(src: &[&[f32]; 4], nc: usize, ycc: bool, out: &mut [u8]) {
    let w = out.len() / nc.max(1);
    let [a, b, c, d] = src.map(|s| if s.len() == w { s } else { &[][..] });
    match (nc, ycc, a.len() == w && (nc < 2 || b.len() == w) && (nc < 3 || c.len() == w) && (nc < 4 || d.len() == w)) {
        (1, _, true) => {
            for (o, &g) in out.iter_mut().zip(a) {
                *o = to_u8(g);
            }
        }
        (3, true, true) => {
            for (o, ((&y, &cb), &cr)) in out.as_chunks_mut::<3>().0.iter_mut().zip(a.iter().zip(b).zip(c)) {
                let [r, g, bl] = ycc_to_rgb(y, cb, cr, 128.0);
                *o = [to_u8(r), to_u8(g), to_u8(bl)];
            }
        }
        (3, false, true) => {
            for (o, ((&r, &g), &bl)) in out.as_chunks_mut::<3>().0.iter_mut().zip(a.iter().zip(b).zip(c)) {
                *o = [to_u8(r), to_u8(g), to_u8(bl)];
            }
        }
        _ => {
            let mut px = [0f32; 4];
            for (x, o) in out.chunks_exact_mut(nc).enumerate() {
                for (p, row) in px.iter_mut().zip(src) {
                    *p = row.get(x).copied().unwrap_or(0.0);
                }
                to_color(&mut px, nc, ycc, 255.0);
                for (o, &p) in o.iter_mut().zip(&px) {
                    *o = to_u8(p);
                }
            }
        }
    }
}

/// The samples of component `c`'s row `r` (in its own coordinates) in `planes`, which hold the
/// MCU lines before `decoded`; empty when that line isn't held, which a band never asks for.
fn plane_row<'p>(b: &Bands<'_>, planes: &'p [Vec<f32>], decoded: usize, c: usize, r: usize) -> &'p [f32] {
    let (Some(comp), Some(plane)) = (b.comps.get(c), planes.get(c)) else { return &[] };
    let rows = comp.vb * 8;
    let (m, k) = (r / rows, r % rows);
    let slots = 2 * b.lag + 1;
    if m >= decoded || m + slots < decoded {
        return &[];
    }
    let at = ((m % slots) * rows + k) * comp.stride;
    plane.get(at..at + comp.stride).unwrap_or_default()
}

#[cfg(test)]
mod tests;
