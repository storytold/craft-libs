//! Lookup decoders for the variable-length codes, built from the `(code, length)` tables in
//! [`super::super::tables`] so that the encoder and the decoder share one copy of Annex B.

use std::sync::OnceLock;

use craft_media::bitstream::BitReader;

use super::super::tables::{self, CBPY, DC_SIZE_CHROMA, DC_SIZE_LUMA, ESCAPE, MCBPC_INTER, MCBPC_INTRA, MVD, Tcoef};
use super::{DecodeError, Result};

/// Longest code of any table here, in bits.
const MAX_BITS: u32 = 12;

/// A prefix code decoded by one table lookup of [`MAX_BITS`] bits.
pub(super) struct Vlc {
    /// `value << 8 | length` for every `MAX_BITS`-bit pattern, 0 where no code matches.
    lut: Vec<u32>,
}

impl Vlc {
    /// `codes` holds `(code, length, value)`; lengths are 1 to [`MAX_BITS`].
    fn new(codes: impl IntoIterator<Item = (u32, u32, u32)>) -> Vlc {
        let mut lut = vec![0u32; 1 << MAX_BITS];
        for (code, len, value) in codes {
            if len == 0 || len > MAX_BITS {
                continue;
            }
            let shift = MAX_BITS - len;
            let first = (code as usize) << shift;
            for slot in lut.iter_mut().skip(first).take(1 << shift) {
                *slot = value << 8 | len;
            }
        }
        Vlc { lut }
    }

    /// Read one code and return its value.
    pub(super) fn read(&self, r: &mut BitReader, what: &'static str) -> Result<u32> {
        let e = self.lut.get(r.peek(MAX_BITS) as usize).copied().unwrap_or(0);
        let len = e & 0xFF;
        if len == 0 || len as usize > r.bits_left() {
            return Err(DecodeError::Invalid(what));
        }
        r.skip(len as usize).map_err(|_| DecodeError::Invalid(what))?;
        Ok(e >> 8)
    }
}

/// `mcbpc` value of the macroblock stuffing code (`0000 0000 1`).
pub(super) const MCBPC_STUFFING: u32 = 0xFF;

fn small(table: &[(u16, u8)]) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
    table.iter().enumerate().map(|(i, &(c, l))| (u32::from(c), u32::from(l), i as u32))
}

/// Table B-6: value `(mb_type - 3) * 4 + cbpc`, or [`MCBPC_STUFFING`].
pub(super) fn mcbpc_intra() -> &'static Vlc {
    static T: OnceLock<Vlc> = OnceLock::new();
    T.get_or_init(|| Vlc::new(small(&MCBPC_INTRA).chain([(1, 9, MCBPC_STUFFING)])))
}

/// Table B-7: value `mb_type * 4 + cbpc`, or [`MCBPC_STUFFING`].
pub(super) fn mcbpc_inter() -> &'static Vlc {
    static T: OnceLock<Vlc> = OnceLock::new();
    T.get_or_init(|| Vlc::new(small(&MCBPC_INTER).chain([(1, 9, MCBPC_STUFFING)])))
}

/// Table B-8: the coded block pattern of an intra macroblock (block 0 in bit 3).
pub(super) fn cbpy() -> &'static Vlc {
    static T: OnceLock<Vlc> = OnceLock::new();
    T.get_or_init(|| Vlc::new(small(&CBPY)))
}

/// Table B-12: the magnitude of `motion_code` (a sign bit follows a non-zero one).
pub(super) fn mvd() -> &'static Vlc {
    static T: OnceLock<Vlc> = OnceLock::new();
    T.get_or_init(|| Vlc::new(small(&MVD)))
}

/// Tables B-13 and B-14: `dct_dc_size`.
pub(super) fn dc_size(luma: bool) -> &'static Vlc {
    static L: OnceLock<Vlc> = OnceLock::new();
    static C: OnceLock<Vlc> = OnceLock::new();
    if luma { L.get_or_init(|| Vlc::new(small(&DC_SIZE_LUMA))) } else { C.get_or_init(|| Vlc::new(small(&DC_SIZE_CHROMA))) }
}

/// The B-VOP `mb_type` (Table B-4): 0 direct, 1 interpolate, 2 backward, 3 forward.
pub(super) fn mb_type_b() -> &'static Vlc {
    static T: OnceLock<Vlc> = OnceLock::new();
    T.get_or_init(|| Vlc::new([(1, 1, 0), (1, 2, 1), (1, 3, 2), (1, 4, 3)]))
}

/// Table B-33: `dmv_length` of a sprite warping vector, 0–14.
pub(super) fn dmv_length() -> &'static Vlc {
    static T: OnceLock<Vlc> = OnceLock::new();
    T.get_or_init(|| {
        let mut codes = vec![(0, 2, 0), (2, 3, 1), (3, 3, 2), (4, 3, 3), (5, 3, 4), (6, 3, 5)];
        // 6 to 14: (n - 3) ones then a zero.
        for v in 6u32..=14 {
            let len = v - 2;
            codes.push(((1 << len) - 2, len, v));
        }
        Vlc::new(codes)
    })
}

/// One TCOEF event: `last`, `run` and the level magnitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Event {
    pub last: bool,
    pub run: u32,
    pub level: u32,
}

/// A TCOEF decoder: the table codes plus the escape, with the table's `LMAX` and `RMAX`.
pub(super) struct TcoefVlc {
    vlc: Vlc,
    table: &'static Tcoef,
}

const ESCAPE_VALUE: u32 = 0xFFFF;

impl TcoefVlc {
    fn build(table: &'static Tcoef) -> TcoefVlc {
        let mut codes = Vec::new();
        for last in [false, true] {
            for run in 0..64u32 {
                for level in 1..32u32 {
                    if let Some((code, len)) = table.code(last, run as usize, level) {
                        codes.push((code, len, u32::from(last) << 12 | run << 6 | level));
                    }
                }
            }
        }
        codes.push((u32::from(ESCAPE.0), u32::from(ESCAPE.1), ESCAPE_VALUE));
        TcoefVlc { vlc: Vlc::new(codes), table }
    }

    fn table_event(&self, r: &mut BitReader) -> Result<Option<(Event, bool)>> {
        let v = self.vlc.read(r, "invalid coefficient code")?;
        if v == ESCAPE_VALUE {
            return Ok(None);
        }
        let ev = Event { last: v >> 12 != 0, run: (v >> 6) & 63, level: v & 63 };
        let neg = r.read_bit().map_err(eof)?;
        Ok(Some((ev, neg)))
    }

    /// Read one event with its sign (§7.4.1.3: the three escape modes).
    pub(super) fn read(&self, r: &mut BitReader) -> Result<(Event, i32)> {
        let signed = |ev: Event, neg: bool| -> (Event, i32) { (ev, if neg { -(ev.level as i32) } else { ev.level as i32 }) };
        if let Some((ev, neg)) = self.table_event(r)? {
            return Ok(signed(ev, neg));
        }
        if !r.read_bit().map_err(eof)? {
            // Mode 1: the level is offset by LMAX.
            let (mut ev, neg) = self.table_event(r)?.ok_or(DecodeError::Invalid("escape inside an escape"))?;
            ev.level += self.table.lmax(ev.last, ev.run as usize);
            return Ok(signed(ev, neg));
        }
        if !r.read_bit().map_err(eof)? {
            // Mode 2: the run is offset by RMAX + 1.
            let (mut ev, neg) = self.table_event(r)?.ok_or(DecodeError::Invalid("escape inside an escape"))?;
            let rmax = self.table.rmax(ev.last, ev.level).ok_or(DecodeError::Invalid("escape level"))?;
            ev.run += rmax as u32 + 1;
            return Ok(signed(ev, neg));
        }
        // Mode 3: fixed length.
        let last = r.read_bit().map_err(eof)?;
        let run = r.read_bits(6).map_err(eof)?;
        marker(r)?;
        let raw = r.read_bits(12).map_err(eof)?;
        marker(r)?;
        let level = ((raw << 20) as i32) >> 20;
        if level == 0 || level == -2048 {
            return Err(DecodeError::Invalid("forbidden escape level"));
        }
        Ok((Event { last, run, level: level.unsigned_abs() }, level))
    }
}

/// Table B-16.
pub(super) fn tcoef_intra() -> &'static TcoefVlc {
    static T: OnceLock<TcoefVlc> = OnceLock::new();
    T.get_or_init(|| TcoefVlc::build(tables::intra()))
}

/// Table B-17.
pub(super) fn tcoef_inter() -> &'static TcoefVlc {
    static T: OnceLock<TcoefVlc> = OnceLock::new();
    T.get_or_init(|| TcoefVlc::build(tables::inter()))
}

pub(super) fn eof(_: craft_media::bitstream::BitError) -> DecodeError {
    DecodeError::Invalid("unexpected end of data")
}

/// A `marker_bit`. Encoders in the wild do not always set it, so like most decoders this one
/// reads it without checking.
pub(super) fn marker(r: &mut BitReader) -> Result<()> {
    r.read_bit().map(|_| ()).map_err(eof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_media::bitstream::BitWriter;

    #[test]
    fn every_small_code_decodes_to_its_index() {
        let check = |vlc: &Vlc, table: &[(u16, u8)]| {
            for (i, &(c, l)) in table.iter().enumerate() {
                let mut w = BitWriter::new();
                w.write_bits(u32::from(c), u32::from(l));
                w.write_bits(0, 16);
                let data = w.finish();
                let mut r = BitReader::new(&data);
                assert_eq!(vlc.read(&mut r, "x").ok(), Some(i as u32));
                assert_eq!(r.position(), usize::from(l));
            }
        };
        check(mcbpc_intra(), &MCBPC_INTRA);
        check(mcbpc_inter(), &MCBPC_INTER);
        check(cbpy(), &CBPY);
        check(mvd(), &MVD);
        check(dc_size(true), &DC_SIZE_LUMA);
        check(dc_size(false), &DC_SIZE_CHROMA);
    }

    #[test]
    fn dmv_length_codes() {
        for (bits, len, v) in [("00", 2, 0), ("010", 3, 1), ("110", 3, 5), ("1110", 4, 6), ("111111111110", 12, 14)] {
            let mut w = BitWriter::new();
            for ch in bits.chars() {
                w.write_bit(ch == '1');
            }
            w.write_bits(0, 16);
            let data = w.finish();
            let mut r = BitReader::new(&data);
            assert_eq!(dmv_length().read(&mut r, "x").ok(), Some(v), "{bits}");
            assert_eq!(r.position(), len);
        }
    }

    #[test]
    fn escape_modes() {
        let t = tcoef_inter();
        let tab = tables::inter();
        // Mode 1: last 0, run 0, level 1 + LMAX(0, 0) = 13, negative.
        let mut w = BitWriter::new();
        w.write_bits(u32::from(ESCAPE.0), u32::from(ESCAPE.1));
        w.write_bit(false);
        let (c, l) = tab.code(false, 0, 1).unwrap_or((0, 0));
        w.write_bits(c, l);
        w.write_bit(true);
        // Mode 2: last 1, level 1, run 0 + RMAX(1, 1) + 1 = 41.
        w.write_bits(u32::from(ESCAPE.0), u32::from(ESCAPE.1));
        w.write_bits(0b10, 2);
        let (c, l) = tab.code(true, 0, 1).unwrap_or((0, 0));
        w.write_bits(c, l);
        w.write_bit(false);
        // Mode 3: last 0, run 5, level -300.
        w.write_bits(u32::from(ESCAPE.0), u32::from(ESCAPE.1));
        w.write_bits(0b11, 2);
        w.write_bit(false);
        w.write_bits(5, 6);
        w.write_bit(true);
        w.write_bits((-300i32 as u32) & 0xFFF, 12);
        w.write_bit(true);
        w.write_bits(0, 16);
        let data = w.finish();
        let mut r = BitReader::new(&data);
        assert_eq!(t.read(&mut r).ok(), Some((Event { last: false, run: 0, level: 13 }, -13)));
        assert_eq!(t.read(&mut r).ok(), Some((Event { last: true, run: 41, level: 1 }, 1)));
        assert_eq!(t.read(&mut r).ok(), Some((Event { last: false, run: 5, level: 300 }, -300)));
    }
}
