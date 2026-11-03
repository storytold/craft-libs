#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room decoder for the JPEG processes most Rust JPEG decoders don't handle, written from
//! ITU-T T.81 (ISO/IEC 10918-1).
//!
//! [`decode`] supports these frames:
//!
//! - sequential DCT, Huffman coded, 8 or 12-bit samples (SOF0 and SOF1; SOF0 files that declare
//!   12 bits are read as SOF1);
//! - extended sequential DCT, arithmetic coded, 8 or 12-bit (SOF9);
//! - progressive DCT, arithmetic coded, 8 or 12-bit (SOF10).
//!
//! Every other frame returns [`Error::Unsupported`]: progressive Huffman (SOF2), lossless (SOF3,
//! SOF11) and hierarchical (SOF5–SOF7, SOF13–SOF15) files, and DCT precisions other than 8 and
//! 12 bits.
//!
//! An app that also uses a baseline decoder routes each file with [`frame`], which reports the
//! first frame marker and its precision without decoding. For example, to send this crate the
//! arithmetic-coded files and the Huffman-coded ones that aren't 8-bit:
//!
//! ```
//! fn use_craft_jpeg(bytes: &[u8]) -> bool {
//!     match craft_jpeg::frame(bytes) {
//!         Some(f) if (0xC9..=0xCB).contains(&f.marker) => true,
//!         Some(f) if matches!(f.marker, 0xC0 | 0xC1) => f.precision != 8,
//!         _ => false,
//!     }
//! }
//! # assert!(!use_craft_jpeg(&[0xFF, 0xD8]));
//! ```
//!
//! Every scan is decoded into a whole-image buffer of quantized coefficients (Annex F for the
//! sequential scans, Annex G for the progressive ones; the arithmetic decoder of Annex D with the
//! conditioning of `DAC`), then dequantized, inverse transformed in floating point (A.3.3),
//! upsampled and color converted. 12-bit images come out as 16-bit samples (scaled, not
//! truncated). Restart intervals, 1, 3 and 4 components and any sampling factors are supported.
//! Metadata (ICC profiles, EXIF, XMP, Photoshop resources, resolution) is left to the caller,
//! which also reads the APP14 `Adobe` segment and passes its transform byte to [`decode`].
//!
//! The decoder never panics on input: every read from the stream is bounds-checked, the work
//! is bounded by 64 passes over the frame's blocks (`SCAN_PASSES`), and
//! allocations are checked against the [`Limits`] first.
//! Damaged entropy-coded data stops that scan; what was decoded is kept and the caller is told
//! through [`Decoded::damaged`].

/// Why a stream could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The stream breaks T.81 in a way the decoder cannot recover from.
    #[error("malformed JPEG: {0}")]
    Malformed(String),
    /// A valid JPEG process or feature this decoder does not implement.
    #[error("{0}")]
    Unsupported(String),
    /// Decoding would exceed the caller's [`Limits`].
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}

fn err(m: impl std::fmt::Display) -> Error {
    Error::Malformed(m.to_string())
}

fn unsupported(m: impl Into<String>) -> Error {
    Error::Unsupported(m.into())
}

/// Decompression-bomb guards, checked against the frame header before any image buffer is
/// allocated.
///
/// ```
/// let mut limits = craft_jpeg::Limits::default();
/// limits.max_alloc = 512 << 20;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Maximum `width * height`.
    pub max_pixels: u64,
    /// Maximum bytes for the decoded output and the decoder's own coefficient and sample
    /// buffers together. Totals above `isize::MAX` are rejected whatever this is set to.
    pub max_alloc: u64,
}

impl Default for Limits {
    fn default() -> Self {
        // 2^30 pixels (e.g. 32768²) and, on 64-bit targets, 8 GiB. On 32-bit targets (wasm32 has
        // a 4 GiB address space shared with the rest of the app) 1 GiB is the most one buffer can
        // safely ask for.
        let max_alloc = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 1 << 30 };
        Limits { max_pixels: 1 << 30, max_alloc }
    }
}

impl Limits {
    /// Check an output buffer of `width × height` pixels of `bytes_per_pixel` bytes.
    fn check(&self, width: usize, height: usize, bytes_per_pixel: u64) -> Result<(), Error> {
        if width == 0 || height == 0 {
            return Err(err(format!("zero-sized image {width}x{height}")));
        }
        let pixels = (width as u64).saturating_mul(height as u64);
        if pixels > self.max_pixels {
            return Err(Error::LimitExceeded(format!("{pixels} pixels exceed max {}", self.max_pixels)));
        }
        let bytes = pixels.saturating_mul(bytes_per_pixel);
        if bytes > self.max_alloc {
            return Err(Error::LimitExceeded(format!("{bytes} bytes exceed max_alloc {}", self.max_alloc)));
        }
        if usize::try_from(bytes).is_err() {
            return Err(Error::LimitExceeded(format!("{bytes} bytes do not fit in memory")));
        }
        Ok(())
    }
}

/// Zigzag scan position `k` → natural (row-major) coefficient index (Figure A.6).
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43,
    36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// The first frame header of a stream, as reported by [`frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameKind {
    /// The second byte of the SOFn marker: `0xC0` (SOF0) to `0xCF` (SOF15), never `0xC4`,
    /// `0xC8` or `0xCC`.
    pub marker: u8,
    /// The sample precision in bits (P in B.2.2), as declared; not validated.
    pub precision: u8,
}

/// The first frame marker (SOFn) and its sample precision, found by walking the marker
/// segments that precede it. `None` when the stream doesn't start with SOI or reaches a scan,
/// EOI or the end of the data first. Reads only the headers, so it is cheap enough to route
/// every JPEG an app opens.
pub fn frame(b: &[u8]) -> Option<FrameKind> {
    if b.get(..2) != Some(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    loop {
        if *b.get(i)? != 0xFF {
            return None;
        }
        let m = *b.get(i + 1)?;
        match m {
            0xFF => i += 1,
            0x01 | 0xD0..=0xD8 => i += 2,
            0xD9 | 0xDA => return None,
            _ => {
                let len = usize::from(u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]));
                if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                    return Some(FrameKind { marker: m, precision: *b.get(i + 4)? });
                }
                i = i.checked_add(2 + len)?;
            }
        }
    }
}

/// Decoded samples, interleaved by pixel (`components` values per pixel), rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Samples {
    /// 8-bit frames: `0..=255`.
    U8(Vec<u8>),
    /// 12-bit frames, scaled to the full 16-bit range: `round(v × 65535 / 4095)`.
    U16(Vec<u16>),
}

/// A decoded image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel: 1 for gray, 3 for RGB, 4 for CMYK.
    ///
    /// Three-component frames are converted from YCbCr to RGB (JFIF, T.871) unless the Adobe
    /// transform is 0 or the component IDs are `R`, `G`, `B`. Four-component samples are ink
    /// amounts (0 = no ink): the stored values are taken to follow Adobe's convention, inverted
    /// (maximum = no ink), and are inverted back. With Adobe transform 2 (YCCK) the first three
    /// components are converted from YCbCr, which yields the ink amounts of C, M and Y directly,
    /// and K is inverted.
    pub components: u8,
    pub samples: Samples,
    /// Entropy-coded data was damaged or cut short: the rest of that scan, and any scans past
    /// the decoder's cap, were left out, and those blocks keep what earlier scans decoded.
    pub damaged: bool,
}

// ---------------------------------------------------------------------------------------------
// Tables and frame

/// A Huffman table in the decoding form of F.2.2.3 (`MAXCODE`, `VALPTR`, `MINCODE`).
#[derive(Clone)]
struct Huffman {
    maxcode: [i32; 17],
    valptr: [i32; 17],
    mincode: [i32; 17],
    values: Vec<u8>,
}

impl Huffman {
    fn new(counts: &[u8], values: &[u8]) -> Result<Self, Error> {
        let mut h = Huffman { maxcode: [-1; 17], valptr: [0; 17], mincode: [0; 17], values: values.to_vec() };
        let mut code: i32 = 0;
        let mut k: i32 = 0;
        for (l, &n) in (1..=16usize).zip(counts) {
            let n = i32::from(n);
            if n > 0 {
                h.valptr[l] = k;
                h.mincode[l] = code;
                code += n;
                k += n;
                h.maxcode[l] = code - 1;
                if code > 1 << l {
                    return Err(err("Huffman table has too many codes"));
                }
            }
            code <<= 1;
        }
        Ok(h)
    }

    fn decode(&self, bits: &mut Bits) -> Result<u8, Error> {
        let mut code: i32 = 0;
        for l in 1..=16 {
            code = (code << 1) | bits.bit() as i32;
            if code <= self.maxcode[l] {
                let idx = self.valptr[l] + code - self.mincode[l];
                return usize::try_from(idx).ok().and_then(|i| self.values.get(i)).copied().ok_or_else(|| err("bad Huffman code"));
            }
        }
        Err(err("bad Huffman code"))
    }
}

struct Tables {
    /// Quantization tables in zigzag order.
    quant: [Option<[u16; 64]>; 4],
    dc: [Option<Huffman>; 4],
    ac: [Option<Huffman>; 4],
    /// Arithmetic DC conditioning (L, U) and AC conditioning (Kx) per table (F.1.4.4; defaults
    /// of B.2.4.3: L = 0, U = 1, Kx = 5).
    dc_l: [u8; 4],
    dc_u: [u8; 4],
    ac_kx: [u8; 4],
    restart: usize,
}

impl Default for Tables {
    fn default() -> Self {
        Tables { quant: [None; 4], dc: Default::default(), ac: Default::default(), dc_l: [0; 4], dc_u: [1; 4], ac_kx: [5; 4], restart: 0 }
    }
}

struct Component {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    /// Samples in this component (B.2.2: ⌈X·H/Hmax⌉ × ⌈Y·V/Vmax⌉).
    cw: usize,
    ch: usize,
    /// Blocks per line and block lines, padded to whole MCUs.
    bw: usize,
    bh: usize,
    /// Quantized coefficients, one block per entry, in zigzag order.
    coefs: Vec<[i32; 64]>,
    /// Table `tq` as it was at the component's first scan. B.2.2 lets a DQT redefine the slot
    /// once every scan of the component is done; libjpeg latches it the same way
    /// (`latch_quant_tables`).
    quant: Option<[u16; 64]>,
}

struct Frame {
    arithmetic: bool,
    progressive: bool,
    precision: u8,
    width: usize,
    height: usize,
    hmax: usize,
    vmax: usize,
    mcux: usize,
    mcuy: usize,
    comps: Vec<Component>,
}

fn be16(s: &[u8], at: usize) -> Result<u16, Error> {
    match (s.get(at), s.get(at + 1)) {
        (Some(&a), Some(&b)) => Ok(u16::from_be_bytes([a, b])),
        _ => Err(err("segment too short")),
    }
}

fn byte(s: &[u8], at: usize) -> Result<u8, Error> {
    s.get(at).copied().ok_or_else(|| err("segment too short"))
}

fn parse_dqt(seg: &[u8], t: &mut Tables) -> Result<(), Error> {
    let mut i = 0;
    while i < seg.len() {
        let pq_tq = byte(seg, i)?;
        let (pq, tq) = (pq_tq >> 4, usize::from(pq_tq & 15));
        i += 1;
        let mut q = [0u16; 64];
        for v in &mut q {
            if pq == 0 {
                *v = u16::from(byte(seg, i)?);
                i += 1;
            } else {
                *v = be16(seg, i)?;
                i += 2;
            }
        }
        let slot = t.quant.get_mut(tq).ok_or_else(|| err("bad quantization table id"))?;
        *slot = Some(q);
    }
    Ok(())
}

fn parse_dht(seg: &[u8], t: &mut Tables) -> Result<(), Error> {
    let mut i = 0;
    while i < seg.len() {
        let tc_th = byte(seg, i)?;
        let counts = seg.get(i + 1..i + 17).ok_or_else(|| err("Huffman table too short"))?;
        let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
        let values = seg.get(i + 17..i + 17 + total).ok_or_else(|| err("Huffman table too short"))?;
        let table = Huffman::new(counts, values)?;
        let slot = match tc_th >> 4 {
            0 => t.dc.get_mut(usize::from(tc_th & 15)),
            1 => t.ac.get_mut(usize::from(tc_th & 15)),
            _ => None,
        };
        *slot.ok_or_else(|| err("bad Huffman table id"))? = Some(table);
        i += 17 + total;
    }
    Ok(())
}

fn parse_dac(seg: &[u8], t: &mut Tables) -> Result<(), Error> {
    for pair in seg.chunks(2) {
        let [tc_tb, cs] = *pair else { return Err(err("DAC segment too short")) };
        let tb = usize::from(tc_tb & 15);
        match tc_tb >> 4 {
            0 => {
                let (l, u) = (cs & 15, cs >> 4);
                if l > u {
                    return Err(err("DAC conditioning has L > U"));
                }
                *t.dc_l.get_mut(tb).ok_or_else(|| err("bad DAC table id"))? = l;
                *t.dc_u.get_mut(tb).ok_or_else(|| err("bad DAC table id"))? = u;
            }
            1 => {
                if !(1..=63).contains(&cs) {
                    return Err(err("DAC Kx out of range"));
                }
                *t.ac_kx.get_mut(tb).ok_or_else(|| err("bad DAC table id"))? = cs;
            }
            _ => return Err(err("bad DAC table class")),
        }
    }
    Ok(())
}

fn parse_frame(marker: u8, seg: &[u8], limits: &Limits) -> Result<Frame, Error> {
    let (arithmetic, progressive) = match marker {
        0xC0 | 0xC1 => (false, false),
        0xC9 => (true, false),
        0xCA => (true, true),
        0xC2 => return Err(unsupported("progressive Huffman-coded JPEG is not supported")),
        0xC3 | 0xCB => return Err(unsupported("lossless JPEG is not supported")),
        _ => return Err(unsupported("hierarchical JPEG is not supported")),
    };
    let precision = byte(seg, 0)?;
    if precision != 8 && precision != 12 {
        return Err(unsupported(format!("{precision}-bit DCT JPEG is not supported")));
    }
    let height = usize::from(be16(seg, 1)?);
    let width = usize::from(be16(seg, 3)?);
    if height == 0 {
        return Err(unsupported("JPEG with the height in a DNL marker is not supported"));
    }
    let nf = usize::from(byte(seg, 5)?);
    if !matches!(nf, 1 | 3 | 4) {
        return Err(unsupported(format!("JPEG with {nf} components is not supported")));
    }
    let mut raw = Vec::with_capacity(nf);
    for c in 0..nf {
        let at = 6 + 3 * c;
        let (id, hv, tq) = (byte(seg, at)?, byte(seg, at + 1)?, byte(seg, at + 2)?);
        let (h, v) = (usize::from(hv >> 4), usize::from(hv & 15));
        if !(1..=4).contains(&h) || !(1..=4).contains(&v) || tq > 3 {
            return Err(err("bad component sampling factors or table"));
        }
        raw.push((id, h, v, usize::from(tq)));
    }
    let hmax = raw.iter().map(|c| c.1).max().unwrap_or(1);
    let vmax = raw.iter().map(|c| c.2).max().unwrap_or(1);
    let bytes_per_pixel = nf as u64 * if precision > 8 { 2 } else { 1 };
    limits.check(width, height, bytes_per_pixel)?;
    let mcux = width.div_ceil(8 * hmax);
    let mcuy = height.div_ceil(8 * vmax);
    // `render` holds the coefficients (4 bytes) and the reconstructed plane (4 bytes) of every
    // padded sample while it writes the output, so all three count against `max_alloc` together.
    let mut total = (width as u64 * height as u64).saturating_mul(bytes_per_pixel);
    for &(_, h, v, _) in &raw {
        let samples = (mcux * h * 8) as u64 * (mcuy * v * 8) as u64;
        total = total.saturating_add(samples.saturating_mul(8));
    }
    // No allocation may pass `isize::MAX` bytes, which matters on 32-bit targets when a caller
    // raises `max_alloc`.
    let cap = limits.max_alloc.min(isize::MAX as u64);
    if total > cap {
        return Err(Error::LimitExceeded(format!("{total} bytes of decoder buffers and output exceed the limit of {cap}")));
    }
    let mut comps = Vec::with_capacity(nf);
    for (id, h, v, tq) in raw {
        let (bw, bh) = (mcux * h, mcuy * v);
        let blocks = bw.checked_mul(bh).ok_or_else(|| Error::LimitExceeded("too many blocks".into()))?;
        comps.push(Component {
            id,
            h,
            v,
            tq,
            cw: (width * h).div_ceil(hmax),
            ch: (height * v).div_ceil(vmax),
            bw,
            bh,
            coefs: vec![[0; 64]; blocks],
            quant: None,
        });
    }
    Ok(Frame { arithmetic, progressive, precision, width, height, hmax, vmax, mcux, mcuy, comps })
}

// ---------------------------------------------------------------------------------------------
// Entropy-coded segments

/// Where the entropy-coded data starting at `start` ends: the first marker that isn't a
/// stuffed `FF 00`, a fill byte or a restart marker.
fn scan_end(b: &[u8], start: usize) -> usize {
    let mut i = start;
    while let Some(p) = b.get(i..).and_then(|r| r.iter().position(|&v| v == 0xFF)) {
        i += p;
        match b.get(i + 1) {
            Some(0x00 | 0xD0..=0xD7) => i += 2,
            Some(0xFF) => i += 1,
            Some(_) => return i,
            None => return b.len(),
        }
    }
    b.len()
}

/// Past the next restart marker at or after `pos`, and `true`; or at the next other marker
/// (or the end) and `false`.
fn skip_restart(d: &[u8], mut pos: usize) -> (usize, bool) {
    while let Some(&v) = d.get(pos) {
        if v != 0xFF {
            pos += 1;
            continue;
        }
        match d.get(pos + 1) {
            Some(0xD0..=0xD7) => return (pos + 2, true),
            Some(0x00) => pos += 2,
            Some(0xFF) => pos += 1,
            _ => return (pos, false),
        }
    }
    (pos, false)
}

/// Next data byte for the decoders: `FF 00` is a data `FF`; at a marker (or the end) zeros
/// are supplied and the marker is not consumed (D.2.6, F.2.2.5).
fn next_byte(d: &[u8], pos: &mut usize) -> Option<u8> {
    let &v = d.get(*pos)?;
    if v != 0xFF {
        *pos += 1;
        return Some(v);
    }
    let mut p = *pos + 1;
    while d.get(p) == Some(&0xFF) {
        p += 1;
    }
    if d.get(p) == Some(&0x00) {
        *pos = p + 1;
        Some(0xFF)
    } else {
        *pos = p - 1;
        None
    }
}

/// Huffman bit reader (F.2.2.5).
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u32,
    n: u32,
    /// How many of the `n` buffered bits (always the last ones) are zero padding supplied at
    /// a marker or the end of the data, not real entropy-coded bits.
    pad: u32,
    /// Set once a read consumed padding: the scan's data ended before the scan did.
    exhausted: bool,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0, acc: 0, n: 0, pad: 0, exhausted: false }
    }

    fn fill(&mut self) {
        while self.n <= 24 {
            let v = match next_byte(self.d, &mut self.pos) {
                Some(v) => v,
                None => {
                    self.pad += 8;
                    0
                }
            };
            self.acc |= u32::from(v) << (24 - self.n);
            self.n += 8;
        }
    }

    fn bit(&mut self) -> u32 {
        self.get(1)
    }

    /// The next `n` (≤ 16) bits.
    fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        if self.n < n {
            self.fill();
        }
        if n > self.n.saturating_sub(self.pad) {
            self.exhausted = true;
        }
        let v = self.acc >> (32 - n);
        self.acc <<= n;
        self.n -= n;
        self.pad = self.pad.min(self.n);
        v
    }

    fn restart(&mut self) {
        self.pos = skip_restart(self.d, self.pos).0;
        self.acc = 0;
        self.n = 0;
        self.pad = 0;
    }
}

/// `EXTEND` of F.2.2.1: the signed value of the `t`-bit magnitude `v`.
fn extend(v: u32, t: u32) -> i32 {
    let v = v as i32;
    if t == 0 {
        0
    } else if v < 1 << (t - 1) {
        v - (1 << t) + 1
    } else {
        v
    }
}

/// Qe, next index after LPS, next index after MPS, MPS switch (Table D.2).
#[rustfmt::skip]
const QE: [(u32, u8, u8, bool); 113] = [
    (0x5A1D, 1, 1, true), (0x2586, 14, 2, false), (0x1114, 16, 3, false), (0x080B, 18, 4, false),
    (0x03D8, 20, 5, false), (0x01DA, 23, 6, false), (0x00E5, 25, 7, false), (0x006F, 28, 8, false),
    (0x0036, 30, 9, false), (0x001A, 33, 10, false), (0x000D, 35, 11, false), (0x0006, 9, 12, false),
    (0x0003, 10, 13, false), (0x0001, 12, 13, false), (0x5A7F, 15, 15, true), (0x3F25, 36, 16, false),
    (0x2CF2, 38, 17, false), (0x207C, 39, 18, false), (0x17B9, 40, 19, false), (0x1182, 42, 20, false),
    (0x0CEF, 43, 21, false), (0x09A1, 45, 22, false), (0x072F, 46, 23, false), (0x055C, 48, 24, false),
    (0x0406, 49, 25, false), (0x0303, 51, 26, false), (0x0240, 52, 27, false), (0x01B1, 54, 28, false),
    (0x0144, 56, 29, false), (0x00F5, 57, 30, false), (0x00B7, 59, 31, false), (0x008A, 60, 32, false),
    (0x0068, 62, 33, false), (0x004E, 63, 34, false), (0x003B, 32, 35, false), (0x002C, 33, 9, false),
    (0x5AE1, 37, 37, true), (0x484C, 64, 38, false), (0x3A0D, 65, 39, false), (0x2EF1, 67, 40, false),
    (0x261F, 68, 41, false), (0x1F33, 69, 42, false), (0x19A8, 70, 43, false), (0x1518, 72, 44, false),
    (0x1177, 73, 45, false), (0x0E74, 74, 46, false), (0x0BFB, 75, 47, false), (0x09F8, 77, 48, false),
    (0x0861, 78, 49, false), (0x0706, 79, 50, false), (0x05CD, 48, 51, false), (0x04DE, 50, 52, false),
    (0x040F, 50, 53, false), (0x0363, 51, 54, false), (0x02D4, 52, 55, false), (0x025C, 53, 56, false),
    (0x01F8, 54, 57, false), (0x01A4, 55, 58, false), (0x0160, 56, 59, false), (0x0125, 57, 60, false),
    (0x00F6, 58, 61, false), (0x00CB, 59, 62, false), (0x00AB, 61, 63, false), (0x008F, 61, 32, false),
    (0x5B12, 65, 65, true), (0x4D04, 80, 66, false), (0x412C, 81, 67, false), (0x37D8, 82, 68, false),
    (0x2FE8, 83, 69, false), (0x293C, 84, 70, false), (0x2379, 86, 71, false), (0x1EDF, 87, 72, false),
    (0x1AA9, 87, 73, false), (0x174E, 72, 74, false), (0x1424, 72, 75, false), (0x119C, 74, 76, false),
    (0x0F6B, 74, 77, false), (0x0D51, 75, 78, false), (0x0BB6, 77, 79, false), (0x0A40, 77, 48, false),
    (0x5832, 80, 81, true), (0x4D1C, 88, 82, false), (0x438E, 89, 83, false), (0x3BDD, 90, 84, false),
    (0x34EE, 91, 85, false), (0x2EAE, 92, 86, false), (0x299A, 93, 87, false), (0x2516, 86, 71, false),
    (0x5570, 88, 89, true), (0x4CA9, 95, 90, false), (0x44D9, 96, 91, false), (0x3E22, 97, 92, false),
    (0x3824, 99, 93, false), (0x32B4, 99, 94, false), (0x2E17, 93, 86, false), (0x56A8, 95, 96, true),
    (0x4F46, 101, 97, false), (0x47E5, 102, 98, false), (0x41CF, 103, 99, false), (0x3C3D, 104, 100, false),
    (0x375E, 99, 93, false), (0x5231, 105, 102, false), (0x4C0F, 106, 103, false), (0x4639, 107, 104, false),
    (0x415E, 103, 99, false), (0x5627, 105, 106, true), (0x50E7, 108, 107, false), (0x4B85, 109, 103, false),
    (0x5597, 110, 109, false), (0x504F, 111, 107, false), (0x5A10, 110, 111, true), (0x5522, 112, 109, false),
    (0x59EB, 112, 111, true),
];

/// The arithmetic decoder of Annex D (D.2): `C` holds `Cx` in its high 16 bits, `A` the
/// interval, `CT` the bits left before the next byte. A statistics bin is one byte: the
/// Qe index in bits 0–6 and the MPS in bit 7.
struct Arith<'a> {
    d: &'a [u8],
    /// `d` ends at a marker rather than at the end of the file.
    at_marker: bool,
    pos: usize,
    c: u32,
    a: u32,
    ct: u32,
    /// The decoder needed bytes past the end of the file.
    eof: bool,
    /// A restart was due but its marker was missing.
    lost: bool,
}

impl<'a> Arith<'a> {
    fn new(d: &'a [u8], at_marker: bool) -> Self {
        let mut ar = Arith { d, at_marker, pos: 0, c: 0, a: 0, ct: 0, eof: false, lost: false };
        ar.init();
        ar
    }

    /// At a marker the decoder is fed zeros for as long as it asks (D.2.6): encoders drop
    /// trailing zero bytes (libjpeg's `finish_pass` does, as D.1.8 allows), and a flat tail of
    /// a few thousand blocks can decode entirely from those zeros. Running out at the end of
    /// the file is damage.
    fn byte_in(&mut self) {
        let v = next_byte(self.d, &mut self.pos).unwrap_or_else(|| {
            let at_restart = matches!(self.d.get(self.pos + 1), Some(0xD0..=0xD7));
            self.eof |= !at_restart && !self.at_marker;
            0
        });
        self.c = self.c.wrapping_add(u32::from(v) << 8);
    }

    /// The data ended without a marker, or a restart marker is missing.
    fn exhausted(&self) -> bool {
        self.lost || self.eof
    }

    /// INITDEC (D.2.7).
    fn init(&mut self) {
        self.a = 0x10000;
        self.c = 0;
        self.byte_in();
        self.c <<= 8;
        self.byte_in();
        self.c <<= 8;
        self.ct = 0;
    }

    /// DECODE (D.2.2–D.2.5) of one binary decision with the adaptive bin `st`.
    fn decode(&mut self, st: &mut u8) -> u8 {
        let mps = *st >> 7;
        let (qe, nlps, nmps, switch) = QE.get(usize::from(*st & 0x7F)).copied().unwrap_or(QE[0]);
        self.a = self.a.wrapping_sub(qe);
        let d;
        if (self.c >> 16) < self.a {
            if self.a >= 0x8000 {
                return mps;
            }
            // Conditional MPS exchange.
            if self.a < qe {
                d = 1 - mps;
                *st = nlps | ((mps ^ u8::from(switch)) << 7);
            } else {
                d = mps;
                *st = nmps | (mps << 7);
            }
        } else {
            // Conditional LPS exchange.
            self.c = self.c.wrapping_sub(self.a << 16);
            if self.a < qe {
                d = mps;
                *st = nmps | (mps << 7);
            } else {
                d = 1 - mps;
                *st = nlps | ((mps ^ u8::from(switch)) << 7);
            }
            self.a = qe;
        }
        // RENORM_D: A ≥ 1 (every Qe is), so this runs at most 16 times.
        while self.a < 0x8000 {
            if self.ct == 0 {
                self.byte_in();
                self.ct = 8;
            }
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
        }
        d
    }

    /// A decision coded with the fixed estimate Qe = 0x5A1D (signs, DC refinement bits).
    fn decode_fixed(&mut self) -> u8 {
        let mut st = 0;
        self.decode(&mut st)
    }

    fn restart(&mut self) {
        let (pos, found) = skip_restart(self.d, self.pos);
        self.pos = pos;
        self.lost |= !found;
        self.init();
    }
}

fn bin(st: &mut [u8], i: usize) -> Result<&mut u8, Error> {
    st.get_mut(i).ok_or_else(|| err("arithmetic statistics index out of range"))
}

/// One scan's decoding of a block, after a restart marker, or at its start.
trait ScanDecoder {
    fn restart(&mut self);
    /// Decode one block of scan component `sc` into `blk` (zigzag order).
    fn block(&mut self, sc: usize, blk: &mut [i32; 64]) -> Result<(), Error>;
}

/// The parameters of a scan header (B.2.3).
struct Scan {
    /// Frame component index of each scan component, with its DC and AC table ids.
    comps: Vec<(usize, usize, usize)>,
    ss: usize,
    se: usize,
    ah: u8,
    al: u8,
}

/// Huffman sequential decoding (F.2.2).
struct HuffmanScan<'a> {
    bits: Bits<'a>,
    tables: Vec<(&'a Huffman, &'a Huffman)>,
    pred: [i32; 4],
}

impl ScanDecoder for HuffmanScan<'_> {
    fn restart(&mut self) {
        self.bits.restart();
        self.pred = [0; 4];
    }

    fn block(&mut self, sc: usize, blk: &mut [i32; 64]) -> Result<(), Error> {
        let &(dc, ac) = self.tables.get(sc).ok_or_else(|| err("bad scan component"))?;
        let pred = self.pred.get_mut(sc).ok_or_else(|| err("bad scan component"))?;
        let t = u32::from(dc.decode(&mut self.bits)?);
        if t > 15 {
            return Err(err("bad DC difference category"));
        }
        let diff = extend(self.bits.get(t), t);
        *pred = pred.wrapping_add(diff);
        blk[0] = *pred;
        let mut k = 1;
        while k < 64 {
            let rs = ac.decode(&mut self.bits)?;
            let (r, s) = (usize::from(rs >> 4), u32::from(rs & 15));
            if s == 0 {
                if r != 15 {
                    break;
                }
                k += 16;
                continue;
            }
            k += r;
            let slot = blk.get_mut(k).ok_or_else(|| err("AC coefficient past the end of the block"))?;
            *slot = extend(self.bits.get(s), s);
            k += 1;
        }
        if self.bits.exhausted {
            return Err(err("entropy-coded data ends before the scan does"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArithMode {
    Sequential,
    DcFirst,
    DcRefine,
    AcFirst,
    AcRefine,
}

/// Arithmetic decoding: sequential (F.2.4) and progressive (G.1.3).
struct ArithScan<'a> {
    ar: Arith<'a>,
    mode: ArithMode,
    ss: usize,
    se: usize,
    al: u8,
    /// DC and AC table ids of each scan component.
    ids: Vec<(usize, usize)>,
    dc_stats: [[u8; 64]; 4],
    ac_stats: [[u8; 256]; 4],
    dc_l: [u8; 4],
    dc_u: [u8; 4],
    ac_kx: [u8; 4],
    /// DC prediction and conditioning context (offset of S0) per scan component.
    pred: [i32; 4],
    ctx: [usize; 4],
}

impl ArithScan<'_> {
    /// Decode_DC_DIFF (F.2.4.1, Figures F.19 and F.22).
    fn dc_diff(&mut self, sc: usize, tbl: usize) -> Result<i32, Error> {
        let st = self.dc_stats.get_mut(tbl).ok_or_else(|| err("bad DC table"))?;
        let ctx = self.ctx.get_mut(sc).ok_or_else(|| err("bad scan component"))?;
        let s0 = *ctx;
        if self.ar.decode(bin(st, s0)?) == 0 {
            *ctx = 0;
            return Ok(0);
        }
        let sign = self.ar.decode(bin(st, s0 + 1)?);
        let mut s = s0 + 2 + usize::from(sign);
        let mut m: i32 = i32::from(self.ar.decode(bin(st, s)?));
        if m != 0 {
            // Magnitude category: X1 … X15 at bins 20 … 34.
            s = 20;
            while self.ar.decode(bin(st, s)?) != 0 {
                m <<= 1;
                s += 1;
                if s > 34 {
                    return Err(err("DC difference out of range"));
                }
            }
        }
        // Magnitude bits: M2 … M15 at bins 35 … 48.
        let mut v = m;
        let mut bit = m >> 1;
        while bit != 0 {
            if self.ar.decode(bin(st, s + 14)?) != 0 {
                v |= bit;
            }
            bit >>= 1;
        }
        let magnitude = v + 1;
        // Conditioning category of the next difference (F.1.4.4.1.2).
        let l = self.dc_l.get(tbl).copied().unwrap_or(0);
        let u = self.dc_u.get(tbl).copied().unwrap_or(1);
        *ctx = if magnitude <= (1 << l) >> 1 {
            0
        } else if magnitude <= 1 << u {
            4 + 4 * usize::from(sign)
        } else {
            12 + 4 * usize::from(sign)
        };
        Ok(if sign != 0 { -magnitude } else { magnitude })
    }

    /// Decode_AC_coefficients (F.2.4.2, Figures F.20 and F.24) over `ss..=se`, scaled by `al`.
    fn ac_first(&mut self, tbl: usize, blk: &mut [i32; 64]) -> Result<(), Error> {
        let kx = usize::from(self.ac_kx.get(tbl).copied().unwrap_or(5));
        let st = self.ac_stats.get_mut(tbl).ok_or_else(|| err("bad AC table"))?;
        let mut k = self.ss.max(1);
        while k <= self.se {
            if self.ar.decode(bin(st, 3 * (k - 1))?) != 0 {
                break; // end of block
            }
            while self.ar.decode(bin(st, 3 * (k - 1) + 1)?) == 0 {
                k += 1;
                if k > self.se {
                    return Err(err("AC coefficient past the end of the band"));
                }
            }
            let sign = self.ar.decode_fixed();
            let sp = 3 * (k - 1) + 2;
            let mut m: i32 = i32::from(self.ar.decode(bin(st, sp)?));
            let mut s = sp;
            if m != 0 && self.ar.decode(bin(st, sp)?) != 0 {
                m = 2;
                s = if k <= kx { 189 } else { 217 };
                let last = s + 13;
                while self.ar.decode(bin(st, s)?) != 0 {
                    m <<= 1;
                    s += 1;
                    if s > last {
                        return Err(err("AC coefficient out of range"));
                    }
                }
            }
            let mut v = m;
            let mut bit = m >> 1;
            while bit != 0 {
                if self.ar.decode(bin(st, s + 14)?) != 0 {
                    v |= bit;
                }
                bit >>= 1;
            }
            let value = if sign != 0 { -(v + 1) } else { v + 1 };
            *blk.get_mut(k).ok_or_else(|| err("bad AC index"))? = value << self.al;
            k += 1;
        }
        Ok(())
    }

    /// Successive approximation of AC coefficients (G.1.3.3, Figure G.10).
    fn ac_refine(&mut self, tbl: usize, blk: &mut [i32; 64]) -> Result<(), Error> {
        let st = self.ac_stats.get_mut(tbl).ok_or_else(|| err("bad AC table"))?;
        let p1: i32 = 1 << self.al;
        let m1: i32 = -1 << self.al;
        let (ss, se) = (self.ss.max(1), self.se);
        // Past the last coefficient already non-zero, an end-of-block decision is coded.
        let eob = (ss..=se).rev().find(|&k| blk.get(k).is_some_and(|&c| c != 0)).map_or(ss, |k| k + 1);
        let mut k = ss;
        while k <= se {
            if k >= eob && self.ar.decode(bin(st, 3 * (k - 1))?) != 0 {
                break;
            }
            loop {
                let c = *blk.get(k).ok_or_else(|| err("bad AC index"))?;
                if c != 0 {
                    if self.ar.decode(bin(st, 3 * (k - 1) + 2)?) != 0 {
                        *blk.get_mut(k).ok_or_else(|| err("bad AC index"))? = if c < 0 { c.wrapping_add(m1) } else { c.wrapping_add(p1) };
                    }
                    break;
                }
                if self.ar.decode(bin(st, 3 * (k - 1) + 1)?) != 0 {
                    *blk.get_mut(k).ok_or_else(|| err("bad AC index"))? = if self.ar.decode_fixed() != 0 { m1 } else { p1 };
                    break;
                }
                k += 1;
                if k > se {
                    return Err(err("AC coefficient past the end of the band"));
                }
            }
            k += 1;
        }
        Ok(())
    }
}

impl ScanDecoder for ArithScan<'_> {
    fn restart(&mut self) {
        self.ar.restart();
        self.reset();
    }

    fn block(&mut self, sc: usize, blk: &mut [i32; 64]) -> Result<(), Error> {
        let &(td, ta) = self.ids.get(sc).ok_or_else(|| err("bad scan component"))?;
        match self.mode {
            ArithMode::Sequential | ArithMode::DcFirst => {
                let diff = self.dc_diff(sc, td)?;
                let pred = self.pred.get_mut(sc).ok_or_else(|| err("bad scan component"))?;
                *pred = pred.wrapping_add(diff);
                blk[0] = pred.wrapping_shl(u32::from(self.al));
                if self.mode == ArithMode::Sequential {
                    self.ac_first(ta, blk)?;
                }
            }
            ArithMode::DcRefine => {
                if self.ar.decode_fixed() != 0 {
                    blk[0] |= 1 << self.al;
                }
            }
            ArithMode::AcFirst => self.ac_first(ta, blk)?,
            ArithMode::AcRefine => self.ac_refine(ta, blk)?,
        }
        if self.ar.exhausted() {
            return Err(err("entropy-coded data ends before the scan does"));
        }
        Ok(())
    }
}

impl ArithScan<'_> {
    fn reset(&mut self) {
        self.dc_stats = [[0; 64]; 4];
        self.ac_stats = [[0; 256]; 4];
        self.pred = [0; 4];
        self.ctx = [0; 4];
    }
}

/// The scan's MCU grid (A.2): MCUs per line, MCU lines, and the blocks the scan decodes.
fn scan_grid(frame: &Frame, scan: &Scan) -> Result<(usize, usize, usize), Error> {
    if let [(ci, _, _)] = scan.comps[..] {
        let c = frame.comps.get(ci).ok_or_else(|| err("bad scan component"))?;
        let (mcux, mcuy) = (c.cw.div_ceil(8), c.ch.div_ceil(8));
        return Ok((mcux, mcuy, mcux.saturating_mul(mcuy)));
    }
    let mut per_mcu = 0usize;
    for &(ci, _, _) in &scan.comps {
        let c = frame.comps.get(ci).ok_or_else(|| err("bad scan component"))?;
        per_mcu += c.h * c.v;
    }
    Ok((frame.mcux, frame.mcuy, frame.mcux.saturating_mul(frame.mcuy).saturating_mul(per_mcu)))
}

/// Walk the scan's MCUs in order (A.2), handing each block to `dec`; counts decoded MCUs.
fn walk_scan(frame: &mut Frame, scan: &Scan, restart: usize, dec: &mut dyn ScanDecoder, done: &mut usize) -> Result<(), Error> {
    let single = scan.comps.len() == 1;
    let (mcux, mcuy, _) = scan_grid(frame, scan)?;
    let total = mcux.saturating_mul(mcuy);
    for m in 0..total {
        if restart > 0 && m > 0 && m % restart == 0 {
            dec.restart();
        }
        let (mx, my) = (m % mcux, m / mcux);
        for (sc, &(ci, _, _)) in scan.comps.iter().enumerate() {
            let comp = frame.comps.get_mut(ci).ok_or_else(|| err("bad scan component"))?;
            let (h, v) = if single { (1, 1) } else { (comp.h, comp.v) };
            for by in my * v..(my + 1) * v {
                for bx in mx * h..(mx + 1) * h {
                    let idx = by * comp.bw + bx;
                    let blk = comp.coefs.get_mut(idx).ok_or_else(|| err("block out of range"))?;
                    dec.block(sc, blk)?;
                }
            }
        }
        *done += 1;
    }
    Ok(())
}

fn parse_scan(seg: &[u8], frame: &Frame) -> Result<Scan, Error> {
    let ns = usize::from(byte(seg, 0)?);
    if !(1..=4).contains(&ns) {
        return Err(err("bad number of scan components"));
    }
    let mut comps = Vec::with_capacity(ns);
    for s in 0..ns {
        let (cs, t) = (byte(seg, 1 + 2 * s)?, byte(seg, 2 + 2 * s)?);
        let ci = frame.comps.iter().position(|c| c.id == cs).ok_or_else(|| err("scan names an unknown component"))?;
        let (td, ta) = (usize::from(t >> 4), usize::from(t & 15));
        if td > 3 || ta > 3 {
            return Err(err("bad entropy table id"));
        }
        comps.push((ci, td, ta));
    }
    let at = 1 + 2 * ns;
    let (ss, se, a) = (usize::from(byte(seg, at)?), usize::from(byte(seg, at + 1)?), byte(seg, at + 2)?);
    let (ah, al) = (a >> 4, a & 15);
    if frame.progressive {
        let ok = ss <= se && se <= 63 && (ss != 0 || se == 0) && (ss == 0 || ns == 1) && al <= 13;
        if !ok {
            return Err(err("bad progressive scan parameters"));
        }
        Ok(Scan { comps, ss, se, ah, al })
    } else {
        Ok(Scan { comps, ss: 0, se: 63, ah: 0, al: 0 })
    }
}

/// Decode one scan's entropy-coded data `d`, counting the decoded MCUs in `done`.
/// `at_marker` says whether a marker follows `d` (otherwise it runs to the end of the file).
fn decode_scan(frame: &mut Frame, tables: &Tables, scan: &Scan, d: &[u8], at_marker: bool, done: &mut usize) -> Result<(), Error> {
    for &(ci, _, _) in &scan.comps {
        let comp = frame.comps.get_mut(ci).ok_or_else(|| err("bad scan component"))?;
        if comp.quant.is_none() {
            let q = tables.quant.get(comp.tq).copied().flatten().ok_or_else(|| err("scan uses an undefined quantization table"))?;
            comp.quant = Some(q);
        }
    }
    if frame.arithmetic {
        let mode = match (frame.progressive, scan.ss, scan.ah) {
            (false, _, _) => ArithMode::Sequential,
            (true, 0, 0) => ArithMode::DcFirst,
            (true, 0, _) => ArithMode::DcRefine,
            (true, _, 0) => ArithMode::AcFirst,
            (true, _, _) => ArithMode::AcRefine,
        };
        let mut dec = ArithScan {
            ar: Arith::new(d, at_marker),
            mode,
            ss: scan.ss,
            se: scan.se,
            al: scan.al,
            ids: scan.comps.iter().map(|&(_, td, ta)| (td, ta)).collect(),
            dc_stats: [[0; 64]; 4],
            ac_stats: [[0; 256]; 4],
            dc_l: tables.dc_l,
            dc_u: tables.dc_u,
            ac_kx: tables.ac_kx,
            pred: [0; 4],
            ctx: [0; 4],
        };
        walk_scan(frame, scan, tables.restart, &mut dec, done)
    } else {
        let mut sel = Vec::with_capacity(scan.comps.len());
        for &(_, td, ta) in &scan.comps {
            let dc = tables.dc.get(td).and_then(Option::as_ref).ok_or_else(|| err("scan uses an undefined Huffman table"))?;
            let ac = tables.ac.get(ta).and_then(Option::as_ref).ok_or_else(|| err("scan uses an undefined Huffman table"))?;
            sel.push((dc, ac));
        }
        let mut dec = HuffmanScan { bits: Bits::new(d), tables: sel, pred: [0; 4] };
        walk_scan(frame, scan, tables.restart, &mut dec, done)
    }
}

/// The blocks decoded from one stream are capped at this many times the frame's blocks; the
/// scan that would pass the cap and every later one are dropped, and the image is reported
/// damaged. Every scan walks its whole MCU grid, so without a cap a small file repeating scan
/// headers multiplies the work by the frame size. 64 passes allow a progressive file one scan
/// per coefficient for every component; real files make about 10.
const SCAN_PASSES: usize = 64;

/// Parse the stream and decode every scan into coefficients.
fn decode_coefficients(b: &[u8], limits: &Limits) -> Result<(Frame, Tables, bool), Error> {
    if b.get(..2) != Some(&[0xFF, 0xD8]) {
        return Err(err("not a JPEG stream"));
    }
    let mut tables = Tables::default();
    let mut frame: Option<Frame> = None;
    let mut damaged = false;
    let mut mcus = 0usize;
    let mut scans = 0usize;
    let mut blocks = 0usize;
    let mut i = 2;
    while let Some(&v) = b.get(i) {
        if v != 0xFF {
            i += 1;
            continue;
        }
        let Some(&m) = b.get(i + 1) else { break };
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
        let Ok(len) = be16(b, i + 2) else { break };
        let len = usize::from(len);
        if len < 2 {
            return Err(err("bad marker segment length"));
        }
        let Some(seg) = b.get(i + 4..i + 2 + len) else {
            if frame.is_some() && scans > 0 {
                damaged = true;
                break;
            }
            return Err(err("marker segment runs past the end of the file"));
        };
        i += 2 + len;
        match m {
            0xDB => parse_dqt(seg, &mut tables)?,
            0xC4 => parse_dht(seg, &mut tables)?,
            0xCC => parse_dac(seg, &mut tables)?,
            0xDD => tables.restart = usize::from(be16(seg, 0)?),
            0xC0..=0xCF => {
                if frame.is_some() {
                    return Err(unsupported("JPEG with more than one frame is not supported"));
                }
                frame = Some(parse_frame(m, seg, limits)?);
            }
            0xDA => {
                let f = frame.as_mut().ok_or_else(|| err("scan before the frame header"))?;
                let scan = parse_scan(seg, f)?;
                let budget = f.comps.iter().map(|c| c.coefs.len()).sum::<usize>().saturating_mul(SCAN_PASSES);
                blocks = blocks.saturating_add(scan_grid(f, &scan)?.2);
                if blocks > budget {
                    damaged = true;
                    break;
                }
                let end = scan_end(b, i);
                let data = b.get(i..end).unwrap_or_default();
                scans += 1;
                if let Err(e) = decode_scan(f, &tables, &scan, data, end < b.len(), &mut mcus) {
                    if mcus == 0 {
                        return Err(e);
                    }
                    damaged = true;
                }
                i = end;
            }
            _ => {}
        }
    }
    let frame = frame.ok_or_else(|| err("no frame header"))?;
    if scans == 0 {
        return Err(err("the file ends before any image data"));
    }
    Ok((frame, tables, damaged))
}

// ---------------------------------------------------------------------------------------------
// Reconstruction

/// `T[x][u] = C(u)/2 · cos((2x+1)uπ/16)`: one dimension of the inverse DCT (A.3.3).
fn idct_basis() -> [[f32; 8]; 8] {
    let mut t = [[0f32; 8]; 8];
    for (x, row) in t.iter_mut().enumerate() {
        for (u, v) in row.iter_mut().enumerate() {
            let c = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
            *v = c / 2.0 * (((2 * x + 1) * u) as f32 * std::f32::consts::PI / 16.0).cos();
        }
    }
    t
}

/// The component's samples (padded to whole blocks, row stride `bw * 8`), level-shifted and
/// clamped to `0..=max`.
fn reconstruct(comp: &Component, q: &[u16; 64], basis: &[[f32; 8]; 8], max: f32) -> Vec<f32> {
    let stride = comp.bw * 8;
    let mut plane = vec![0f32; stride * comp.bh * 8];
    let shift = (max + 1.0) / 2.0;
    for (n, blk) in comp.coefs.iter().enumerate() {
        let (bx, by) = (n % comp.bw, n / comp.bw);
        if blk[1..].iter().all(|&c| c == 0) {
            // Only DC: every sample is T[y][0] · T[x][0] · F[0][0], the same products the full
            // transform below sums (its other terms are zero), so the result is identical.
            let t = basis[0][0];
            let v = (t * (t * (blk[0] as f32 * f32::from(q[0]))) + shift).clamp(0.0, max);
            for y in 0..8 {
                let row = (by * 8 + y) * stride + bx * 8;
                if let Some(out) = plane.get_mut(row..row + 8) {
                    out.fill(v);
                }
            }
            continue;
        }
        let mut f = [0f32; 64];
        for (k, &c) in blk.iter().enumerate() {
            f[ZIGZAG[k]] = c as f32 * f32::from(q[k]);
        }
        // Rows: r[v][x] = Σu T[x][u] F[v][u]; then columns: s[y][x] = Σv T[y][v] r[v][x].
        let mut r = [0f32; 64];
        for v in 0..8 {
            for x in 0..8 {
                r[v * 8 + x] = (0..8).map(|u| basis[x][u] * f[v * 8 + u]).sum();
            }
        }
        for (y, ty) in basis.iter().enumerate() {
            let row = (by * 8 + y) * stride + bx * 8;
            if let Some(out) = plane.get_mut(row..row + 8) {
                for (x, o) in out.iter_mut().enumerate() {
                    let s: f32 = ty.iter().enumerate().map(|(v, t)| t * r[v * 8 + x]).sum();
                    *o = (s + shift).clamp(0.0, max);
                }
            }
        }
    }
    plane
}

/// For each output coordinate: the two neighboring component samples and the weight of the
/// second (centered linear interpolation of a subsampled component).
fn upsample_map(out: usize, samples: usize, factor: usize, fmax: usize) -> Vec<(usize, usize, f32)> {
    let last = samples.saturating_sub(1);
    (0..out)
        .map(|x| {
            let s = ((x as f32 + 0.5) * factor as f32 / fmax as f32 - 0.5).max(0.0);
            let x0 = (s.floor() as usize).min(last);
            ((x0), (x0 + 1).min(last), (s - x0 as f32).clamp(0.0, 1.0))
        })
        .collect()
}

/// Turn the coefficients into interleaved output samples.
fn render(frame: Frame, tables: &Tables, adobe_transform: Option<u8>) -> Result<(u8, Samples), Error> {
    let max = if frame.precision > 8 { 4095.0 } else { 255.0 };
    let basis = idct_basis();
    let (w, h) = (frame.width, frame.height);
    let mut planes = Vec::with_capacity(frame.comps.len());
    for comp in &frame.comps {
        // A component no scan reached has only zero coefficients; any defined table will do.
        let q = comp.quant.or_else(|| tables.quant.get(comp.tq).copied().flatten());
        let q = q.ok_or_else(|| err("component uses an undefined quantization table"))?;
        let plane = reconstruct(comp, &q, &basis, max);
        let xs = upsample_map(w, comp.cw, comp.h, frame.hmax);
        let ys = upsample_map(h, comp.ch, comp.v, frame.vmax);
        planes.push((plane, comp.bw * 8, xs, ys));
    }
    let nc = planes.len();
    let sixteen = frame.precision > 8;
    let rgb_ids = frame.comps.iter().map(|c| c.id).eq(*b"RGB");
    let ycc = match nc {
        3 => adobe_transform != Some(0) && !rgb_ids,
        4 => adobe_transform == Some(2),
        _ => false,
    };
    let (mut wide, mut narrow) = (Vec::new(), Vec::new());
    if sixteen {
        wide.reserve_exact(w * h * nc);
    } else {
        narrow.reserve_exact(w * h * nc);
    }
    let mut px = [0f32; 4];
    let center = (max + 1.0) / 2.0;
    for y in 0..h {
        for x in 0..w {
            for (c, (plane, stride, xs, ys)) in planes.iter().enumerate() {
                let (Some(&(x0, x1, fx)), Some(&(y0, y1, fy))) = (xs.get(x), ys.get(y)) else {
                    return Err(err("sample map out of range"));
                };
                let at = |xx: usize, yy: usize| plane.get(yy * stride + xx).copied().unwrap_or(0.0);
                let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
                let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
                if let Some(p) = px.get_mut(c) {
                    *p = top + (bottom - top) * fy;
                }
            }
            if ycc {
                // YCbCr → RGB (JFIF / T.871); for YCCK the result is the inverted CMY of Adobe's
                // convention, i.e. the ink amounts.
                let (yy, cb, cr) = (px[0], px[1] - center, px[2] - center);
                px[0] = yy + 1.402 * cr;
                px[1] = yy - 0.344_136 * cb - 0.714_136 * cr;
                px[2] = yy + 1.772 * cb;
                if nc == 4 {
                    px[3] = max - px[3];
                }
            } else if nc == 4 {
                // Adobe-style CMYK is stored inverted (max = no ink).
                for p in &mut px {
                    *p = max - *p;
                }
            }
            for &p in px.iter().take(nc) {
                let p = p.clamp(0.0, max);
                if sixteen {
                    wide.push((p * 65535.0 / max).round() as u16);
                } else {
                    narrow.push(p.round() as u8);
                }
            }
        }
    }
    let samples = if sixteen { Samples::U16(wide) } else { Samples::U8(narrow) };
    Ok((nc as u8, samples))
}

/// Decode a JPEG stream of one of the frame types listed in the [crate docs](crate).
///
/// `adobe_transform` is the transform byte of the stream's APP14 `Adobe` segment, if the caller
/// found one: it selects the color conversion of 3 and 4-component images (see
/// [`Decoded::components`]). The decoder ignores every APPn segment itself.
///
/// Damaged entropy-coded data after at least one decoded MCU returns `Ok` with
/// [`Decoded::damaged`] set; a stream that yields no image data returns an error.
pub fn decode(bytes: &[u8], limits: &Limits, adobe_transform: Option<u8>) -> Result<Decoded, Error> {
    let (frame, tables, damaged) = decode_coefficients(bytes, limits)?;
    let (width, height) = (frame.width as u32, frame.height as u32);
    let (components, samples) = render(frame, &tables, adobe_transform)?;
    Ok(Decoded { width, height, components, samples, damaged })
}

#[cfg(test)]
mod tests;
