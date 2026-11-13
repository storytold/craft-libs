//! Encoder-side variable-length codes, built from the decoder's code lists (H.262 Annex B) so the
//! two cannot disagree.

use std::sync::OnceLock;

use crate::mpeg2::vlc::{
    CBP, DC_CHROMA, DC_LUMA, DCT_EOB, DCT_ESCAPE, DCT_ONE, DCT_ZERO, DCT_ZERO_LONG, MBA, MBA_ESCAPE, MBTYPE_I, MBTYPE_P, MOTION, dct_table,
    parse_code,
};

use super::bitwriter::BitWriter;

/// A code: `len` bits of `bits`, most significant first.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct Code {
    pub bits: u32,
    pub len: u8,
}

impl Code {
    fn of(s: &str) -> Code {
        let (bits, len) = parse_code(s);
        Code { bits, len: len as u8 }
    }

    #[inline(always)]
    pub fn put(self, w: &mut BitWriter) {
        w.put(self.bits, u32::from(self.len));
    }
}

/// Longest run with a table code (table zero and one: run 31 at level 1).
const MAX_RUN: usize = 31;
/// Largest level with a table code (run 0: level 40).
const MAX_LEVEL: usize = 40;

pub(crate) struct EncTables {
    /// macroblock_address_increment 1..=33 (index 0 unused) and the escape.
    pub mba: [Code; 34],
    pub mba_escape: Code,
    /// macroblock_type by flags: I pictures, P pictures (index: flags, 0..32).
    pub mbtype_i: [Option<Code>; 32],
    pub mbtype_p: [Option<Code>; 32],
    /// coded_block_pattern_420 (the first six blocks).
    pub cbp: [Code; 64],
    /// motion_code -16..=16 at index code + 16.
    pub motion: [Code; 33],
    pub dc_luma: [Code; 12],
    pub dc_chroma: [Code; 12],
    /// DCT coefficients `[table][run][level]` without the sign bit (table 0: B.14, table 1: B.15).
    pub dct: [Box<[[Option<Code>; MAX_LEVEL + 1]; MAX_RUN + 1]>; 2],
    pub dct_eob: [Code; 2],
    pub dct_escape: Code,
}

fn by_value<const N: usize>(list: &[(&str, i16)], offset: i16) -> [Code; N] {
    let mut out = [Code::default(); N];
    for &(s, v) in list {
        if let Some(slot) = usize::try_from(v + offset).ok().and_then(|i| out.get_mut(i)) {
            *slot = Code::of(s);
        }
    }
    out
}

fn mb_types(list: &[(&str, i16)]) -> [Option<Code>; 32] {
    let mut out = [None; 32];
    for &(s, v) in list {
        if let Some(slot) = usize::try_from(v).ok().and_then(|i| out.get_mut(i)) {
            *slot = Some(Code::of(s));
        }
    }
    out
}

fn dct(
    short: &'static [(&'static str, i16)],
    extra: &'static [(&'static str, i16)],
) -> (Box<[[Option<Code>; MAX_LEVEL + 1]; MAX_RUN + 1]>, Code, Code) {
    let mut t = Box::new([[None; MAX_LEVEL + 1]; MAX_RUN + 1]);
    let (mut eob, mut esc) = (Code::default(), Code::default());
    for (s, v) in dct_table(short, extra) {
        match v {
            DCT_EOB => eob = Code::of(s),
            DCT_ESCAPE => esc = Code::of(s),
            v => {
                let (run, level) = ((v & 63) as usize, (v >> 6) as usize);
                if let Some(slot) = t.get_mut(run).and_then(|r| r.get_mut(level)) {
                    *slot = Some(Code::of(s));
                }
            }
        }
    }
    (t, eob, esc)
}

pub(crate) fn enc_tables() -> &'static EncTables {
    static T: OnceLock<EncTables> = OnceLock::new();
    T.get_or_init(|| {
        let (zero, eob0, esc) = dct(DCT_ZERO, DCT_ZERO_LONG);
        let (one, eob1, _) = dct(DCT_ONE, &[]);
        let mba_escape = MBA.iter().find(|e| e.1 == MBA_ESCAPE).map_or(Code::default(), |e| Code::of(e.0));
        EncTables {
            mba: by_value(MBA, 0),
            mba_escape,
            mbtype_i: mb_types(MBTYPE_I),
            mbtype_p: mb_types(MBTYPE_P),
            cbp: by_value(CBP, 0),
            motion: by_value(MOTION, 16),
            dc_luma: by_value(DC_LUMA, 0),
            dc_chroma: by_value(DC_CHROMA, 0),
            dct: [zero, one],
            dct_eob: [eob0, eob1],
            dct_escape: esc,
        }
    })
}

impl EncTables {
    /// One DCT coefficient. `first_inter`: the first coefficient of a non-intra block, where
    /// run 0 level ±1 is "1s".
    #[inline]
    pub fn put_coefficient(&self, w: &mut BitWriter, table: usize, run: usize, level: i32, first_inter: bool) {
        let mag = level.unsigned_abs() as usize;
        let neg = level < 0;
        if first_inter && run == 0 && mag == 1 {
            w.put(0b10 | neg as u32, 2);
            return;
        }
        if let Some(c) = self.dct[table & 1].get(run).and_then(|r| r.get(mag)).copied().flatten() {
            c.put(w);
            w.bit(neg);
            return;
        }
        // escape: 6-bit run, 12-bit two's complement level (-2047..=2047)
        self.dct_escape.put(w);
        w.put(run as u32, 6);
        w.put((level.clamp(-2047, 2047) & 0xFFF) as u32, 12);
    }

    /// Bits of one motion vector component delta (`delta` already in the wrapped range).
    #[inline]
    pub fn motion_bits(&self, delta: i32, f_code: u8) -> u32 {
        let r_size = u32::from(f_code.saturating_sub(1));
        if delta == 0 {
            return u32::from(self.motion[16].len);
        }
        let code = ((delta.abs() - 1) >> r_size) + 1;
        let idx = (16 + code.min(16)) as usize;
        u32::from(self.motion[idx].len) + r_size
    }

    /// Write one motion vector component delta (`delta` in the wrapped range).
    #[inline]
    pub fn put_motion(&self, w: &mut BitWriter, delta: i32, f_code: u8) {
        let r_size = u32::from(f_code.saturating_sub(1));
        if delta == 0 {
            self.motion[16].put(w);
            return;
        }
        let a = delta.abs() - 1;
        let code = ((a >> r_size) + 1).min(16);
        let idx = (16 + code * delta.signum()) as usize;
        self.motion[idx].put(w);
        w.put((a & ((1 << r_size) - 1)) as u32, r_size);
    }
}

/// `mv - pred` brought into the range a motion code can carry ([-16f, 16f-1], §7.6.3.1).
#[inline]
pub(crate) fn wrap_delta(mv: i32, pred: i32, f_code: u8) -> i32 {
    let f = 1i32 << f_code.saturating_sub(1);
    let d = mv - pred;
    if d > 16 * f - 1 {
        d - 32 * f
    } else if d < -16 * f {
        d + 32 * f
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpeg2::bits::Bits;
    use crate::mpeg2::slice::wrap_vector;
    use crate::mpeg2::vlc::{MB_FWD, MB_INTRA, MB_PATTERN, MB_QUANT, rl, tables};

    /// Macroblock types the encoder sends (frame pictures, frame prediction, no B pictures).
    const ENC_MB_TYPES: [i16; 7] =
        [MB_INTRA, MB_INTRA | MB_QUANT, MB_FWD | MB_PATTERN, MB_PATTERN, MB_FWD, MB_QUANT | MB_FWD | MB_PATTERN, MB_QUANT | MB_PATTERN];

    #[test]
    fn every_code_decodes_to_its_value() {
        let t = enc_tables();
        let d = tables();
        let check = |c: Code, vlc: &crate::mpeg2::vlc::Vlc, v: i16| {
            let mut w = BitWriter::new();
            c.put(&mut w);
            let bytes = w.finish();
            let mut b = Bits::new(&bytes);
            assert_eq!(vlc.decode(&mut b), Some(v));
        };
        for n in 1..=33 {
            check(t.mba[n], &d.mba, n as i16);
        }
        check(t.mba_escape, &d.mba, MBA_ESCAPE);
        for v in 0..64 {
            check(t.cbp[v], &d.cbp, v as i16);
        }
        for v in -16..=16i16 {
            check(t.motion[(v + 16) as usize], &d.motion, v);
        }
        for s in 0..12 {
            check(t.dc_luma[s], &d.dc_luma, s as i16);
            check(t.dc_chroma[s], &d.dc_chroma, s as i16);
        }
        for f in ENC_MB_TYPES {
            // the I table holds only the intra types
            if let Some(c) = t.mbtype_i[f as usize] {
                check(c, &d.mbtype[0], f);
            }
            check(t.mbtype_p[f as usize].unwrap_or_default(), &d.mbtype[1], f);
        }
        for (k, vlc) in [&d.dct_zero, &d.dct_one].into_iter().enumerate() {
            for run in 0..=MAX_RUN {
                for level in 1..=MAX_LEVEL {
                    if let Some(c) = t.dct[k][run][level] {
                        check(c, vlc, rl(run as i16, level as i16));
                    }
                }
            }
            check(t.dct_eob[k], vlc, DCT_EOB);
        }
    }

    #[test]
    fn motion_deltas_round_trip_through_the_decoder_wrap() {
        let t = enc_tables();
        let d = tables();
        for f_code in 1..=4u8 {
            let f = 1i32 << (f_code - 1);
            for pred in [-16 * f, -3, 0, 5, 16 * f - 1] {
                for mv in -16 * f..16 * f {
                    let delta = wrap_delta(mv, pred, f_code);
                    let mut w = BitWriter::new();
                    t.put_motion(&mut w, delta, f_code);
                    let bits = w.bits() as u32;
                    assert_eq!(bits, t.motion_bits(delta, f_code));
                    let bytes = w.finish();
                    let mut b = Bits::new(&bytes);
                    // the decoder's reading of motion_code and motion_residual
                    let code = d.motion.decode(&mut b).unwrap_or(99) as i32;
                    let r_size = (f_code - 1) as u32;
                    let residual = if f_code != 1 && code != 0 { b.read(r_size) as i32 } else { 0 };
                    let dd = if f == 1 || code == 0 { code } else { ((code.abs() - 1) * f + residual + 1) * code.signum() };
                    assert_eq!(dd, delta);
                    assert_eq!(wrap_vector(pred, dd, f), mv, "f_code {f_code} pred {pred} mv {mv}");
                }
            }
        }
    }
}
