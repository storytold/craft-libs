//! Marker segment parsing: quantisation, Huffman and arithmetic tables and the frame header.

use super::huffman::*;
use super::idct::UpsampleStep;
use super::*;

pub(super) struct Tables {
    /// Quantization tables in zigzag order.
    pub(super) quant: [Option<[u16; 64]>; 4],
    pub(super) dc: [Option<Huffman>; 4],
    pub(super) ac: [Option<Huffman>; 4],
    /// Arithmetic DC conditioning (L, U) and AC conditioning (Kx) per table (F.1.4.4; defaults
    /// of B.2.4.3: L = 0, U = 1, Kx = 5).
    pub(super) dc_l: [u8; 4],
    pub(super) dc_u: [u8; 4],
    pub(super) ac_kx: [u8; 4],
    pub(super) restart: usize,
}

impl Default for Tables {
    fn default() -> Self {
        Tables { quant: [None; 4], dc: Default::default(), ac: Default::default(), dc_l: [0; 4], dc_u: [1; 4], ac_kx: [5; 4], restart: 0 }
    }
}

pub(super) struct Component {
    pub(super) id: u8,
    pub(super) h: usize,
    pub(super) v: usize,
    pub(super) tq: usize,
    /// Samples in this component (B.2.2: ⌈X·H/Hmax⌉ × ⌈Y·V/Vmax⌉).
    pub(super) cw: usize,
    pub(super) ch: usize,
    /// Blocks per line and block lines, padded to whole MCUs.
    pub(super) bw: usize,
    pub(super) bh: usize,
    /// Quantized coefficients, one block per entry, in zigzag order.
    pub(super) coefs: Vec<[i32; 64]>,
    /// Table `tq` as it was at the component's first scan. B.2.2 lets a DQT redefine the slot
    /// once every scan of the component is done; libjpeg latches it the same way
    /// (`latch_quant_tables`).
    pub(super) quant: Option<[u16; 64]>,
}

pub(super) struct Frame {
    pub(super) arithmetic: bool,
    pub(super) progressive: bool,
    pub(super) precision: u8,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) hmax: usize,
    pub(super) vmax: usize,
    pub(super) mcux: usize,
    pub(super) mcuy: usize,
    pub(super) comps: Vec<Component>,
}

pub(super) fn be16(s: &[u8], at: usize) -> Result<u16, Error> {
    match (s.get(at), s.get(at + 1)) {
        (Some(&a), Some(&b)) => Ok(u16::from_be_bytes([a, b])),
        _ => Err(err("segment too short")),
    }
}

pub(super) fn byte(s: &[u8], at: usize) -> Result<u8, Error> {
    s.get(at).copied().ok_or_else(|| err("segment too short"))
}

pub(super) fn parse_dqt(seg: &[u8], t: &mut Tables) -> Result<(), Error> {
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

pub(super) fn parse_dht(seg: &[u8], t: &mut Tables) -> Result<(), Error> {
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

pub(super) fn parse_dac(seg: &[u8], t: &mut Tables) -> Result<(), Error> {
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

/// A frame header (B.2.2), validated, before any buffer is sized from it.
pub(super) struct Header {
    pub(super) arithmetic: bool,
    pub(super) progressive: bool,
    pub(super) precision: u8,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) hmax: usize,
    pub(super) vmax: usize,
    /// MCUs per line and MCU lines of an interleaved scan (A.2.3).
    pub(super) mcux: usize,
    pub(super) mcuy: usize,
    /// Identifier, sampling factors (H, V) and quantization table of each component.
    pub(super) comps: Vec<(u8, usize, usize, usize)>,
}

impl Header {
    /// Samples of a component with sampling factors `h`, `v` (B.2.2: ⌈X·H/Hmax⌉ × ⌈Y·V/Vmax⌉).
    pub(super) fn samples(&self, h: usize, v: usize) -> (usize, usize) {
        ((self.width * h).div_ceil(self.hmax), (self.height * v).div_ceil(self.vmax))
    }
}

/// The frame header (B.2.2) of a SOF segment: its size, precision and components, and the MCU
/// grid they make. Sizes no image buffer, so callers can check limits first.
pub(super) fn parse_header(marker: u8, seg: &[u8]) -> Result<Header, Error> {
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
    let mcux = width.div_ceil(8 * hmax);
    let mcuy = height.div_ceil(8 * vmax);
    Ok(Header { arithmetic, progressive, precision, width, height, hmax, vmax, mcux, mcuy, comps: raw })
}

/// The frame of a whole-image decode: the header checked against `limits`, with a coefficient
/// buffer for every block of every component.
pub(super) fn parse_frame(marker: u8, seg: &[u8], limits: &Limits) -> Result<Frame, Error> {
    let hd = parse_header(marker, seg)?;
    let (width, height, mcux, mcuy) = (hd.width, hd.height, hd.mcux, hd.mcuy);
    let bytes_per_pixel = hd.comps.len() as u64 * if hd.precision > 8 { 2 } else { 1 };
    limits.check(width, height, bytes_per_pixel)?;
    // `render` holds the coefficients (4 bytes) and the reconstructed plane (4 bytes) of every
    // padded sample, and a column and a row map for upsampling each component, while it writes
    // the output, so all of them count against `max_alloc` together.
    let mut total = (width as u64 * height as u64).saturating_mul(bytes_per_pixel);
    let maps = (width as u64 + height as u64).saturating_mul(size_of::<UpsampleStep>() as u64);
    for &(_, h, v, _) in &hd.comps {
        let samples = (mcux * h * 8) as u64 * (mcuy * v * 8) as u64;
        total = total.saturating_add(samples.saturating_mul(8)).saturating_add(maps);
    }
    // No allocation may pass `isize::MAX` bytes, which matters on 32-bit targets when a caller
    // raises `max_alloc`.
    let cap = limits.max_alloc.min(isize::MAX as u64);
    if total > cap {
        return Err(Error::LimitExceeded(format!("{total} bytes of decoder buffers and output exceed the limit of {cap}")));
    }
    let mut comps = Vec::with_capacity(hd.comps.len());
    for &(id, h, v, tq) in &hd.comps {
        let (bw, bh) = (mcux * h, mcuy * v);
        let blocks = bw.checked_mul(bh).ok_or_else(|| Error::LimitExceeded("too many blocks".into()))?;
        let (cw, ch) = hd.samples(h, v);
        comps.push(Component { id, h, v, tq, cw, ch, bw, bh, coefs: vec![[0; 64]; blocks], quant: None });
    }
    let Header { arithmetic, progressive, precision, hmax, vmax, .. } = hd;
    Ok(Frame { arithmetic, progressive, precision, width, height, hmax, vmax, mcux, mcuy, comps })
}
