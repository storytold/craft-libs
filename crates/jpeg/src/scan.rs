//! Scan decoding: entropy segment bounds, the scan walk over the MCU grid and the coefficient pass loop.

use super::arith::*;
use super::huffman::*;
use super::parse::*;
use super::*;

/// Where the entropy-coded data starting at `start` ends: the first marker that isn't a
/// stuffed `FF 00`, a fill byte or a restart marker.
pub(super) fn scan_end(b: &[u8], start: usize) -> usize {
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
pub(super) fn skip_restart(d: &[u8], mut pos: usize) -> (usize, bool) {
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
pub(super) fn next_byte(d: &[u8], pos: &mut usize) -> Option<u8> {
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

/// One scan's decoding of a block, after a restart marker, or at its start.
pub(super) trait ScanDecoder {
    fn restart(&mut self);
    /// Decode one block of scan component `sc` into `blk` (zigzag order).
    fn block(&mut self, sc: usize, blk: &mut [i32; 64]) -> Result<(), Error>;
}

/// The parameters of a scan header (B.2.3).
pub(super) struct Scan {
    /// Frame component index of each scan component, with its DC and AC table ids.
    pub(super) comps: Vec<(usize, usize, usize)>,
    pub(super) ss: usize,
    pub(super) se: usize,
    pub(super) ah: u8,
    pub(super) al: u8,
}

/// The scan's MCU grid (A.2): MCUs per line, MCU lines, and the blocks the scan decodes.
pub(super) fn scan_grid(frame: &Frame, scan: &Scan) -> Result<(usize, usize, usize), Error> {
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
pub(super) fn walk_scan(frame: &mut Frame, scan: &Scan, restart: usize, dec: &mut dyn ScanDecoder, done: &mut usize) -> Result<(), Error> {
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

/// A scan header (B.2.3) of a frame whose components have the identifiers `ids`.
pub(super) fn parse_scan(seg: &[u8], ids: &[u8], progressive: bool) -> Result<Scan, Error> {
    let ns = usize::from(byte(seg, 0)?);
    if !(1..=4).contains(&ns) {
        return Err(err("bad number of scan components"));
    }
    let mut comps = Vec::with_capacity(ns);
    for s in 0..ns {
        let (cs, t) = (byte(seg, 1 + 2 * s)?, byte(seg, 2 + 2 * s)?);
        let ci = ids.iter().position(|&id| id == cs).ok_or_else(|| err("scan names an unknown component"))?;
        let (td, ta) = (usize::from(t >> 4), usize::from(t & 15));
        if td > 3 || ta > 3 {
            return Err(err("bad entropy table id"));
        }
        comps.push((ci, td, ta));
    }
    let at = 1 + 2 * ns;
    let (ss, se, a) = (usize::from(byte(seg, at)?), usize::from(byte(seg, at + 1)?), byte(seg, at + 2)?);
    let (ah, al) = (a >> 4, a & 15);
    if progressive {
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
pub(super) fn decode_scan(frame: &mut Frame, tables: &Tables, scan: &Scan, d: &[u8], at_marker: bool, done: &mut usize) -> Result<(), Error> {
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
pub(super) const SCAN_PASSES: usize = 64;

/// Parse the stream and decode every scan into coefficients.
pub(super) fn decode_coefficients(b: &[u8], limits: &Limits) -> Result<(Frame, Tables, bool), Error> {
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
                let ids: Vec<u8> = f.comps.iter().map(|c| c.id).collect();
                let scan = parse_scan(seg, &ids, f.progressive)?;
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
