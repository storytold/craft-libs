//! The ADPCM of Flash (SWF) audio, 2 to 5 bits per sample.
//!
//! The stream starts with two bits, the code size minus 2. It continues in blocks of 4096 samples
//! per channel: each block starts, for every channel, with the first sample (16 bits, signed) and
//! the step index (6 bits, 0 to 88), then 4095 codes per channel, interleaved by sample. All fields
//! are packed most significant bit first. A code is a sign bit and `bits - 1` magnitude bits; the
//! decoder adds `step >> (bits - 1)` plus `step >> k` for each magnitude bit set `k` places below
//! the top one, negated when the sign is set, and moves the step index by the adjustment table of
//! the code size (negative for small magnitudes, positive for large ones). The last block may
//! be shorter.

use super::{Error, Result, clip16, step_at};
use craft_media::bitstream::{BitReader, BitWriter};

/// Samples per channel in a full block.
pub const BLOCK_SAMPLES: usize = 4096;

/// The index adjustment for each magnitude, per code size.
fn adjust(bits: u32) -> &'static [i32] {
    match bits {
        2 => &[-1, 2],
        3 => &[-1, -1, 2, 4],
        4 => &[-1, -1, -1, -1, 2, 4, 6, 8],
        _ => &[-1, -1, -1, -1, -1, -1, -1, -1, 1, 2, 4, 6, 8, 10, 13, 16],
    }
}

#[derive(Clone, Copy)]
struct State {
    predictor: i32,
    index: i32,
}

impl State {
    fn apply(&mut self, code: u32, bits: u32) -> i16 {
        let step = step_at(self.index);
        let mag = code & ((1 << (bits - 1)) - 1);
        let mut diff = step >> (bits - 1);
        for k in 0..bits - 1 {
            if mag & (1 << (bits - 2 - k)) != 0 {
                diff += step >> k;
            }
        }
        if code & (1 << (bits - 1)) != 0 {
            diff = -diff;
        }
        self.predictor = clip16(self.predictor + diff);
        self.index = (self.index + adjust(bits).get(mag as usize).copied().unwrap_or(0)).clamp(0, 88);
        self.predictor as i16
    }

    fn encode(&mut self, sample: i16, bits: u32) -> u32 {
        let step = step_at(self.index);
        let mut diff = i32::from(sample) - self.predictor;
        let mut code = 0u32;
        if diff < 0 {
            code = 1 << (bits - 1);
            diff = -diff;
        }
        let mut s = step;
        for k in 0..bits - 1 {
            if diff >= s {
                code |= 1 << (bits - 2 - k);
                diff -= s;
            }
            s >>= 1;
        }
        self.apply(code, bits);
        code
    }
}

/// Decode a stream. `channels` is 1 or 2; the code size comes from the stream's first two bits.
/// The result holds every sample up to the end of the data (a last block cut short decodes as
/// far as it goes).
pub fn decode(data: &[u8], channels: usize) -> Result<Vec<i16>> {
    if !(1..=2).contains(&channels) {
        return Err(Error::Malformed("Flash ADPCM has one or two channels"));
    }
    let mut r = BitReader::new(data);
    let bits = r.read_bits(2).map_err(|_| Error::Malformed("empty stream"))? + 2;
    let mut out = Vec::new();
    let header_bits = channels * 22;
    while r.bits_left() >= header_bits {
        let mut states = Vec::with_capacity(channels);
        for _ in 0..channels {
            let sample = r.read_bits(16).map_err(|_| Error::Malformed("truncated block"))? as u16 as i16;
            let index = r.read_bits(6).map_err(|_| Error::Malformed("truncated block"))? as i32;
            if index > 88 {
                return Err(Error::Malformed("step index above 88"));
            }
            out.push(sample);
            states.push(State { predictor: i32::from(sample), index });
        }
        'block: for _ in 1..BLOCK_SAMPLES {
            if r.bits_left() < channels * (bits as usize) {
                break 'block;
            }
            for st in states.iter_mut() {
                match r.read_bits(bits) {
                    Ok(code) => out.push(st.apply(code, bits)),
                    Err(_) => break 'block,
                }
            }
        }
    }
    Ok(out)
}

/// Encode interleaved samples with `bits` (2 to 5) bits per sample. Blocks hold 4096 samples per
/// channel; the stream ends at a byte boundary.
pub fn encode(samples: &[i16], channels: usize, bits: u32) -> Result<Vec<u8>> {
    if !(1..=2).contains(&channels) {
        return Err(Error::InvalidInput("Flash ADPCM has one or two channels"));
    }
    if !(2..=5).contains(&bits) {
        return Err(Error::InvalidInput("Flash ADPCM codes have 2 to 5 bits"));
    }
    if !samples.len().is_multiple_of(channels) {
        return Err(Error::InvalidInput("the sample count is not a multiple of the channel count"));
    }
    let mut w = BitWriter::new();
    w.write_bits(bits - 2, 2);
    for block in samples.chunks(BLOCK_SAMPLES * channels) {
        let mut states = Vec::with_capacity(channels);
        for ch in 0..channels {
            let col: Vec<i16> = block.iter().skip(ch).step_by(channels).copied().collect();
            let first = col.first().copied().unwrap_or(0);
            // the index whose first codes follow the signal most closely
            let mut best = (i64::MAX, 0);
            for index in 0..=63 {
                let mut st = State { predictor: i32::from(first), index };
                let mut err = 0i64;
                for &s in col.iter().skip(1).take(16) {
                    st.encode(s, bits);
                    err += (i64::from(s) - i64::from(st.predictor)).pow(2);
                }
                if err < best.0 {
                    best = (err, index);
                }
            }
            w.write_bits(u32::from(first as u16), 16);
            w.write_bits(best.1 as u32, 6);
            states.push(State { predictor: i32::from(first), index: best.1 });
        }
        for frame in block.chunks(channels).skip(1) {
            for (st, &s) in states.iter_mut().zip(frame) {
                w.write_bits(st.encode(s, bits), bits);
            }
        }
    }
    w.align_zero();
    Ok(w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_checked() {
        assert!(encode(&[0], 3, 4).is_err());
        assert!(encode(&[0], 1, 1).is_err());
        assert!(encode(&[0], 1, 6).is_err());
        assert!(decode(&[], 1).is_err());
        assert!(decode(&[0], 0).is_err());
    }
}
