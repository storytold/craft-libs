//! The SBR bitstream (ISO/IEC 14496-3 §4.4.2.8 syntax, §4.5.2.8 semantics): the header and the
//! single and channel pair elements of an `EXT_SBR_DATA` fill element.

use std::sync::OnceLock;

use craft_media::bitstream::BitReader;

use super::freq::FreqTables;
use super::tables::{
    F_ENV_1_5DB, F_ENV_3_0DB, F_ENV_BAL_1_5DB, F_ENV_BAL_3_0DB, T_ENV_1_5DB, T_ENV_3_0DB, T_ENV_BAL_1_5DB, T_ENV_BAL_3_0DB, T_NOISE_3_0DB,
    T_NOISE_BAL_3_0DB,
};
use super::{Header, MAX_ENV};
use crate::aac::huffman::Lut;
use crate::aac::{Error, Result};

/// `bs_frame_class`: fixed leading and trailing frame borders.
pub(crate) const FIXFIX: u8 = 0;
/// Fixed leading border, variable trailing border.
pub(crate) const FIXVAR: u8 = 1;
/// Variable leading border, fixed trailing border.
pub(crate) const VARFIX: u8 = 2;
/// Variable leading and trailing borders.
pub(crate) const VARVAR: u8 = 3;

/// SBR time slots per frame (`numTimeSlots`; two QMF slots each).
const TIME_SLOTS: usize = 16;

/// Decoding tables for the SBR Huffman codes, in [`Book`] order.
fn luts() -> &'static [Lut; 10] {
    static LUTS: OnceLock<[Lut; 10]> = OnceLock::new();
    LUTS.get_or_init(|| {
        [
            Lut::build(&T_ENV_1_5DB),
            Lut::build(&F_ENV_1_5DB),
            Lut::build(&T_ENV_BAL_1_5DB),
            Lut::build(&F_ENV_BAL_1_5DB),
            Lut::build(&T_ENV_3_0DB),
            Lut::build(&F_ENV_3_0DB),
            Lut::build(&T_ENV_BAL_3_0DB),
            Lut::build(&F_ENV_BAL_3_0DB),
            Lut::build(&T_NOISE_3_0DB),
            Lut::build(&T_NOISE_BAL_3_0DB),
        ]
    })
}

/// One of the SBR Huffman codes, and its largest absolute value.
#[derive(Clone, Copy)]
struct Book(usize, i32);

const T_ENV_15: Book = Book(0, 60);
const F_ENV_15: Book = Book(1, 60);
const T_BAL_15: Book = Book(2, 24);
const F_BAL_15: Book = Book(3, 24);
const T_ENV_30: Book = Book(4, 31);
const F_ENV_30: Book = Book(5, 31);
const T_BAL_30: Book = Book(6, 12);
const F_BAL_30: Book = Book(7, 12);
const T_NOISE: Book = Book(8, 31);
const T_NOISE_BAL: Book = Book(9, 12);

fn huff(br: &mut BitReader, book: Book) -> Result<i32> {
    Ok(luts()[book.0].decode(br)? as i32 - book.1)
}

fn bits(br: &mut BitReader, n: u32) -> Result<usize> {
    Ok(br.read_bits(n)? as usize)
}

impl Header {
    /// `sbr_header()`, with the defaults of the fields its extra parts leave out.
    pub(crate) fn read(br: &mut BitReader) -> Result<Header> {
        let amp_res = br.read_flag()?;
        let start_freq = br.read_bits(4)? as u8;
        let stop_freq = br.read_bits(4)? as u8;
        let xover_band = br.read_bits(3)? as u8;
        br.skip(2)?; // bs_reserved
        let (extra_1, extra_2) = (br.read_flag()?, br.read_flag()?);
        let mut h = Header {
            amp_res,
            start_freq,
            stop_freq,
            xover_band,
            freq_scale: 2,
            alter_scale: true,
            noise_bands: 2,
            limiter_bands: 2,
            limiter_gains: 2,
            interpol_freq: true,
            smoothing_mode: true,
        };
        if extra_1 {
            h.freq_scale = br.read_bits(2)? as u8;
            h.alter_scale = br.read_flag()?;
            h.noise_bands = br.read_bits(2)? as u8;
        }
        if extra_2 {
            h.limiter_bands = br.read_bits(2)? as u8;
            h.limiter_gains = br.read_bits(2)? as u8;
            h.interpol_freq = br.read_flag()?;
            h.smoothing_mode = br.read_flag()?;
        }
        Ok(h)
    }
}

/// The time/frequency grid of one channel in one frame (`sbr_grid()`, §4.6.18.3.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Grid {
    pub(crate) class: u8,
    /// Envelopes (1 to 5).
    pub(crate) num_env: usize,
    /// Envelope borders in time slots; `t_env[num_env]` may reach past the frame (up to 19).
    pub(crate) t_env: [usize; MAX_ENV + 1],
    /// Noise floors (1 or 2).
    pub(crate) num_noise: usize,
    /// Noise floor borders.
    pub(crate) t_noise: [usize; 3],
    /// High frequency resolution per envelope.
    pub(crate) freq_res: [bool; MAX_ENV],
    /// The transient envelope `l_A`, if any (it may be `num_env`: the next frame's first).
    pub(crate) la: Option<usize>,
    /// 3 dB envelope steps (the header's choice, but 1.5 dB for a single FIXFIX envelope).
    pub(crate) amp_res: bool,
}

/// `ceil(log2(num_env + 1))`: the width of `bs_pointer`.
fn pointer_bits(num_env: usize) -> u32 {
    usize::BITS - num_env.leading_zeros()
}

fn read_rel(br: &mut BitReader, n: usize) -> Result<[usize; 3]> {
    let mut rel = [0; 3];
    for r in rel.iter_mut().take(n) {
        *r = 2 * bits(br, 2)? + 2;
    }
    Ok(rel)
}

impl Grid {
    fn read(br: &mut BitReader, amp_res: bool) -> Result<Grid> {
        let bad = || Error::Bitstream("invalid SBR time grid");
        let class = br.read_bits(2)? as u8;
        let mut t = [0i64; MAX_ENV + 1];
        let mut freq_res = [false; MAX_ENV];
        let (n, pointer) = match class {
            FIXFIX => {
                let n = 1usize << br.read_bits(2)?;
                if n > 4 {
                    return Err(bad());
                }
                freq_res = [br.read_flag()?; MAX_ENV];
                for (l, t) in t.iter_mut().enumerate().take(n + 1) {
                    *t = (l * TIME_SLOTS / n) as i64;
                }
                (n, 0)
            }
            FIXVAR | VARFIX => {
                let abs = bits(br, 2)?;
                let rels = bits(br, 2)?;
                let n = rels + 1;
                let rel = read_rel(br, rels)?;
                let pointer = bits(br, pointer_bits(n))?;
                for l in 0..n {
                    freq_res[if class == FIXVAR { n - 1 - l } else { l }] = br.read_flag()?;
                }
                if class == FIXVAR {
                    t[n] = (abs + TIME_SLOTS) as i64;
                    for l in (1..n).rev() {
                        t[l] = t[l + 1] - rel[n - 1 - l] as i64;
                    }
                } else {
                    (t[0], t[n]) = (abs as i64, TIME_SLOTS as i64);
                    for l in 1..n {
                        t[l] = t[l - 1] + rel[l - 1] as i64;
                    }
                }
                (n, pointer)
            }
            _ => {
                let abs0 = bits(br, 2)?;
                let abs1 = bits(br, 2)? + TIME_SLOTS;
                let (rels0, rels1) = (bits(br, 2)?, bits(br, 2)?);
                let n = rels0 + rels1 + 1;
                if n > MAX_ENV {
                    return Err(bad());
                }
                let (rel0, rel1) = (read_rel(br, rels0)?, read_rel(br, rels1)?);
                let pointer = bits(br, pointer_bits(n))?;
                for r in freq_res.iter_mut().take(n) {
                    *r = br.read_flag()?;
                }
                (t[0], t[n]) = (abs0 as i64, abs1 as i64);
                for l in 1..=rels0 {
                    t[l] = t[l - 1] + rel0[l - 1] as i64;
                }
                for l in (rels0 + 1..n).rev() {
                    t[l] = t[l + 1] - rel1[n - 1 - l] as i64;
                }
                (n, pointer)
            }
        };
        if t[0] < 0 || t[..=n].windows(2).any(|w| w[0] >= w[1]) {
            return Err(bad());
        }
        // l_A: in ISO/IEC 14496-3 a VARFIX pointer of 1, like 0, marks no transient envelope
        let la = match (pointer, class) {
            (0, _) | (1, VARFIX) => None,
            (p, FIXVAR | VARVAR) => Some((n + 1).checked_sub(p).ok_or_else(bad)?),
            (p, _) => Some(p - 1),
        };
        if la.is_some_and(|la| la > n) {
            return Err(bad());
        }
        let t_env: [usize; MAX_ENV + 1] = std::array::from_fn(|i| t[i] as usize);
        let (num_noise, t_noise) = if n == 1 {
            (1, [t_env[0], t_env[1], 0])
        } else {
            let middle = match (class, pointer) {
                (FIXFIX, _) => n / 2,
                (VARFIX, 0) => 1,
                (VARFIX, 1) => n - 1,
                (VARFIX, p) => p - 1,
                (_, 0 | 1) => n - 1,
                (_, p) => (n + 1).checked_sub(p).ok_or_else(bad)?,
            };
            if middle == 0 || middle >= n {
                return Err(bad());
            }
            (2, [t_env[0], t_env[middle], t_env[n]])
        };
        Ok(Grid { class, num_env: n, t_env, num_noise, t_noise, freq_res, la, amp_res: amp_res && !(class == FIXFIX && n == 1) })
    }
}

/// The data of one channel as transmitted: delta-coded envelope and noise floor values (the
/// first frequency-coded value of each is absolute), not yet scaled for a balance channel.
#[derive(Clone, Debug)]
pub(crate) struct ChannelData {
    pub(crate) grid: Grid,
    pub(crate) df_env: [bool; MAX_ENV],
    pub(crate) df_noise: [bool; 2],
    pub(crate) invf: [u8; 5],
    pub(crate) env: [[i32; 64]; MAX_ENV],
    pub(crate) noise: [[i32; 5]; 2],
    /// `bs_add_harmonic` per high-resolution band, when `bs_add_harmonic_flag` is set.
    pub(crate) harmonic: Option<[bool; 64]>,
}

impl ChannelData {
    fn new(grid: Grid) -> ChannelData {
        ChannelData {
            grid,
            df_env: [false; MAX_ENV],
            df_noise: [false; 2],
            invf: [0; 5],
            env: [[0; 64]; MAX_ENV],
            noise: [[0; 5]; 2],
            harmonic: None,
        }
    }

    fn read_dtdf(&mut self, br: &mut BitReader) -> Result<()> {
        for d in self.df_env.iter_mut().take(self.grid.num_env) {
            *d = br.read_flag()?;
        }
        for d in self.df_noise.iter_mut().take(self.grid.num_noise) {
            *d = br.read_flag()?;
        }
        Ok(())
    }

    fn read_invf(&mut self, br: &mut BitReader, t: &FreqTables) -> Result<()> {
        for m in self.invf.iter_mut().take(t.n_q()) {
            *m = br.read_bits(2)? as u8;
        }
        Ok(())
    }

    /// `sbr_envelope()`; `balance` for the second channel of a coupled pair.
    fn read_envelope(&mut self, br: &mut BitReader, t: &FreqTables, balance: bool) -> Result<()> {
        let (t_book, f_book, start_bits) = match (self.grid.amp_res, balance) {
            (false, false) => (T_ENV_15, F_ENV_15, 7),
            (false, true) => (T_BAL_15, F_BAL_15, 6),
            (true, false) => (T_ENV_30, F_ENV_30, 6),
            (true, true) => (T_BAL_30, F_BAL_30, 5),
        };
        for l in 0..self.grid.num_env {
            let n = t.bands(self.grid.freq_res[l]).len() - 1;
            let row = &mut self.env[l];
            if self.df_env[l] {
                for v in row.iter_mut().take(n) {
                    *v = huff(br, t_book)?;
                }
            } else {
                row[0] = br.read_bits(start_bits)? as i32;
                for v in row.iter_mut().take(n).skip(1) {
                    *v = huff(br, f_book)?;
                }
            }
        }
        Ok(())
    }

    /// `sbr_noise()`.
    fn read_noise(&mut self, br: &mut BitReader, t: &FreqTables, balance: bool) -> Result<()> {
        let (t_book, f_book) = if balance { (T_NOISE_BAL, F_BAL_30) } else { (T_NOISE, F_ENV_30) };
        for l in 0..self.grid.num_noise {
            let row = &mut self.noise[l];
            if self.df_noise[l] {
                for v in row.iter_mut().take(t.n_q()) {
                    *v = huff(br, t_book)?;
                }
            } else {
                row[0] = br.read_bits(5)? as i32;
                for v in row.iter_mut().take(t.n_q()).skip(1) {
                    *v = huff(br, f_book)?;
                }
            }
        }
        Ok(())
    }

    /// `bs_add_harmonic_flag` and `sbr_sinusoidal_coding()`.
    fn read_harmonics(&mut self, br: &mut BitReader, t: &FreqTables) -> Result<()> {
        if br.read_flag()? {
            let mut h = [false; 64];
            for v in h.iter_mut().take(t.n_high()) {
                *v = br.read_flag()?;
            }
            self.harmonic = Some(h);
        }
        Ok(())
    }
}

/// Skip `bs_extended_data` (parametric stereo, for one, is not decoded).
fn skip_extended(br: &mut BitReader) -> Result<()> {
    if br.read_flag()? {
        let mut count = bits(br, 4)?;
        if count == 15 {
            count += bits(br, 8)?;
        }
        br.skip(8 * count)?;
    }
    Ok(())
}

/// The SBR data of one channel element.
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    /// The pair codes its second channel as a balance against the first.
    pub(crate) coupling: bool,
    pub(crate) channels: Vec<ChannelData>,
}

/// `sbr_single_channel_element()` (`channels` 1) or `sbr_channel_pair_element()` (2).
pub(crate) fn read_data(br: &mut BitReader, t: &FreqTables, amp_res: bool, channels: usize) -> Result<Frame> {
    if br.read_flag()? {
        br.skip(4 * channels)?; // bs_reserved
    }
    if channels == 1 {
        let mut c = ChannelData::new(Grid::read(br, amp_res)?);
        c.read_dtdf(br)?;
        c.read_invf(br, t)?;
        c.read_envelope(br, t, false)?;
        c.read_noise(br, t, false)?;
        c.read_harmonics(br, t)?;
        skip_extended(br)?;
        return Ok(Frame { coupling: false, channels: vec![c] });
    }
    let coupling = br.read_flag()?;
    let (mut a, mut b);
    if coupling {
        a = ChannelData::new(Grid::read(br, amp_res)?);
        b = ChannelData::new(a.grid.clone());
        a.read_dtdf(br)?;
        b.read_dtdf(br)?;
        a.read_invf(br, t)?;
        b.invf = a.invf;
        a.read_envelope(br, t, false)?;
        a.read_noise(br, t, false)?;
        b.read_envelope(br, t, true)?;
        b.read_noise(br, t, true)?;
    } else {
        a = ChannelData::new(Grid::read(br, amp_res)?);
        b = ChannelData::new(Grid::read(br, amp_res)?);
        a.read_dtdf(br)?;
        b.read_dtdf(br)?;
        a.read_invf(br, t)?;
        b.read_invf(br, t)?;
        a.read_envelope(br, t, false)?;
        b.read_envelope(br, t, false)?;
        a.read_noise(br, t, false)?;
        b.read_noise(br, t, false)?;
    }
    a.read_harmonics(br, t)?;
    b.read_harmonics(br, t)?;
    skip_extended(br)?;
    Ok(Frame { coupling, channels: vec![a, b] })
}

#[cfg(test)]
mod tests {
    use craft_media::bitstream::BitWriter;

    use super::*;

    /// Every SBR Huffman table is a complete prefix code, with the shortest codeword for a zero
    /// delta, and its decoder reads back every codeword.
    #[test]
    fn huffman_tables_are_complete_prefix_codes() {
        let tables: [&[(u32, u8)]; 10] = [
            &T_ENV_1_5DB,
            &F_ENV_1_5DB,
            &T_ENV_BAL_1_5DB,
            &F_ENV_BAL_1_5DB,
            &T_ENV_3_0DB,
            &F_ENV_3_0DB,
            &T_ENV_BAL_3_0DB,
            &F_ENV_BAL_3_0DB,
            &T_NOISE_3_0DB,
            &T_NOISE_BAL_3_0DB,
        ];
        let lavs = [60, 60, 24, 24, 31, 31, 12, 12, 31, 12];
        for (i, (codes, lav)) in tables.iter().zip(lavs).enumerate() {
            assert_eq!(codes.len(), 2 * lav + 1, "table {i}");
            let max = u32::from(codes.iter().map(|c| c.1).max().unwrap());
            assert_eq!(codes.iter().map(|&(_, l)| 1u64 << (max - u32::from(l))).sum::<u64>(), 1 << max, "table {i}: Kraft sum");
            let shortest = codes.iter().map(|c| c.1).min().unwrap();
            assert_eq!(codes[lav].1, shortest, "table {i}");
            for (sym, &(code, len)) in codes.iter().enumerate() {
                let mut bw = BitWriter::new();
                bw.write_bits(code, u32::from(len));
                bw.write_bits(0x5A5A5A, 24);
                let data = bw.finish();
                let mut br = BitReader::new(&data);
                assert_eq!(huff(&mut br, Book(i, lav as i32)).unwrap(), sym as i32 - lav as i32, "table {i}");
                assert_eq!(br.position(), usize::from(len));
            }
        }
    }

    fn grid(bits: &[(u32, u32)]) -> Result<Grid> {
        let mut bw = BitWriter::new();
        for &(v, n) in bits {
            bw.write_bits(v, n);
        }
        bw.write_bits(0, 16);
        let data = bw.finish();
        Grid::read(&mut BitReader::new(&data), true)
    }

    /// The four frame classes give the borders, noise floors and transient envelope of
    /// §4.6.18.3.3; a FIXFIX frame of one envelope uses 1.5 dB steps.
    #[test]
    fn grids_of_each_frame_class() {
        // FIXFIX, 4 envelopes, high resolution
        let g = grid(&[(FIXFIX as u32, 2), (2, 2), (1, 1)]).unwrap();
        assert_eq!((g.num_env, &g.t_env[..5], g.num_noise, g.t_noise, g.la, g.amp_res), (4, &[0, 4, 8, 12, 16][..], 2, [0, 8, 16], None, true));
        let g = grid(&[(FIXFIX as u32, 2), (0, 2), (0, 1)]).unwrap();
        assert_eq!((g.num_env, g.t_noise, g.amp_res), (1, [0, 16, 0], false));
        // FIXVAR: trailing border 16 + 2, two relative borders (4, 2) back from it, pointer 1,
        // resolutions sent last envelope first
        let g = grid(&[(FIXVAR as u32, 2), (2, 2), (2, 2), (1, 2), (0, 2), (1, 2), (0, 1), (1, 1), (1, 1)]).unwrap();
        assert_eq!((g.num_env, &g.t_env[..4]), (3, &[0, 12, 14, 18][..]));
        assert_eq!((g.la, g.t_noise, g.freq_res[..3].to_vec()), (Some(3), [0, 14, 18], vec![true, true, false]));
        // VARFIX: leading border 1, relative 2, pointer 2: l_A = 1
        let g = grid(&[(VARFIX as u32, 2), (1, 2), (1, 2), (0, 2), (2, 2), (1, 1), (0, 1)]).unwrap();
        assert_eq!((g.num_env, &g.t_env[..3], g.la, g.t_noise), (2, &[1, 3, 16][..], Some(1), [1, 3, 16]));
        // the same with pointer 1: no transient envelope, the noise floors still split at 3
        let g = grid(&[(VARFIX as u32, 2), (1, 2), (1, 2), (0, 2), (1, 2), (1, 1), (0, 1)]).unwrap();
        assert_eq!((g.la, g.t_noise), (None, [1, 3, 16]));
        // VARVAR: 1 .. 17 with one border from each end
        let g = grid(&[(VARVAR as u32, 2), (1, 2), (1, 2), (1, 2), (1, 2), (0, 2), (0, 2), (0, 2), (0, 1), (0, 1), (0, 1)]).unwrap();
        assert_eq!((g.num_env, &g.t_env[..4], g.la), (3, &[1, 3, 15, 17][..], None));
    }

    /// Grids whose borders do not increase, too many envelopes and a pointer past the envelopes
    /// are refused.
    #[test]
    fn invalid_grids_are_refused() {
        // FIXFIX with 8 envelopes
        assert!(grid(&[(FIXFIX as u32, 2), (3, 2), (0, 1)]).is_err());
        // VARVAR with 7 envelopes
        assert!(grid(&[(VARVAR as u32, 2), (0, 2), (0, 2), (3, 2), (3, 2)]).is_err());
        // FIXVAR whose relative borders run back past the start: 16 - (8 + 8 + 8)
        assert!(grid(&[(FIXVAR as u32, 2), (0, 2), (3, 2), (3, 2), (3, 2), (3, 2), (0, 3), (0, 4)]).is_err());
        // VARFIX, 2 envelopes, pointer 3: the middle border would be the trailing one
        assert!(grid(&[(VARFIX as u32, 2), (0, 2), (1, 2), (0, 2), (3, 2), (0, 1), (0, 1)]).is_err());
    }
}
