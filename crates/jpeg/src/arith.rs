//! Arithmetic entropy decoding: the probability table, the QM decoder and the arithmetic scan decoder.

use super::scan::*;
use super::*;

/// Qe, next index after LPS, next index after MPS, MPS switch (Table D.2).
#[rustfmt::skip]
pub(super) const QE: [(u32, u8, u8, bool); 113] = [
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
pub(super) struct Arith<'a> {
    pub(super) d: &'a [u8],
    /// `d` ends at a marker rather than at the end of the file.
    pub(super) at_marker: bool,
    pub(super) pos: usize,
    pub(super) c: u32,
    pub(super) a: u32,
    pub(super) ct: u32,
    /// The decoder needed bytes past the end of the file.
    pub(super) eof: bool,
    /// A restart was due but its marker was missing.
    pub(super) lost: bool,
}

impl<'a> Arith<'a> {
    pub(super) fn new(d: &'a [u8], at_marker: bool) -> Self {
        let mut ar = Arith { d, at_marker, pos: 0, c: 0, a: 0, ct: 0, eof: false, lost: false };
        ar.init();
        ar
    }

    /// At a marker the decoder is fed zeros for as long as it asks (D.2.6): encoders drop
    /// trailing zero bytes (libjpeg's `finish_pass` does, as D.1.8 allows), and a flat tail of
    /// a few thousand blocks can decode entirely from those zeros. Running out at the end of
    /// the file is damage.
    pub(super) fn byte_in(&mut self) {
        let v = next_byte(self.d, &mut self.pos).unwrap_or_else(|| {
            let at_restart = matches!(self.d.get(self.pos + 1), Some(0xD0..=0xD7));
            self.eof |= !at_restart && !self.at_marker;
            0
        });
        self.c = self.c.wrapping_add(u32::from(v) << 8);
    }

    /// The data ended without a marker, or a restart marker is missing.
    pub(super) fn exhausted(&self) -> bool {
        self.lost || self.eof
    }

    /// INITDEC (D.2.7).
    pub(super) fn init(&mut self) {
        self.a = 0x10000;
        self.c = 0;
        self.byte_in();
        self.c <<= 8;
        self.byte_in();
        self.c <<= 8;
        self.ct = 0;
    }

    /// DECODE (D.2.2–D.2.5) of one binary decision with the adaptive bin `st`.
    pub(super) fn decode(&mut self, st: &mut u8) -> u8 {
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
    pub(super) fn decode_fixed(&mut self) -> u8 {
        let mut st = 0;
        self.decode(&mut st)
    }

    pub(super) fn restart(&mut self) {
        let (pos, found) = skip_restart(self.d, self.pos);
        self.pos = pos;
        self.lost |= !found;
        self.init();
    }
}

pub(super) fn bin(st: &mut [u8], i: usize) -> Result<&mut u8, Error> {
    st.get_mut(i).ok_or_else(|| err("arithmetic statistics index out of range"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ArithMode {
    Sequential,
    DcFirst,
    DcRefine,
    AcFirst,
    AcRefine,
}

/// Arithmetic decoding: sequential (F.2.4) and progressive (G.1.3).
pub(super) struct ArithScan<'a> {
    pub(super) ar: Arith<'a>,
    pub(super) mode: ArithMode,
    pub(super) ss: usize,
    pub(super) se: usize,
    pub(super) al: u8,
    /// DC and AC table ids of each scan component.
    pub(super) ids: Vec<(usize, usize)>,
    pub(super) dc_stats: [[u8; 64]; 4],
    pub(super) ac_stats: [[u8; 256]; 4],
    pub(super) dc_l: [u8; 4],
    pub(super) dc_u: [u8; 4],
    pub(super) ac_kx: [u8; 4],
    /// DC prediction and conditioning context (offset of S0) per scan component.
    pub(super) pred: [i32; 4],
    pub(super) ctx: [usize; 4],
}

impl ArithScan<'_> {
    /// Decode_DC_DIFF (F.2.4.1, Figures F.19 and F.22).
    pub(super) fn dc_diff(&mut self, sc: usize, tbl: usize) -> Result<i32, Error> {
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
    pub(super) fn ac_first(&mut self, tbl: usize, blk: &mut [i32; 64]) -> Result<(), Error> {
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
    pub(super) fn ac_refine(&mut self, tbl: usize, blk: &mut [i32; 64]) -> Result<(), Error> {
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
    pub(super) fn reset(&mut self) {
        self.dc_stats = [[0; 64]; 4];
        self.ac_stats = [[0; 256]; 4];
        self.pred = [0; 4];
        self.ctx = [0; 4];
    }
}
