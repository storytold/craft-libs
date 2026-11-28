//! IMA ADPCM in WAV blocks (format tag 0x0011).
//!
//! A block is, per channel, a 4-byte header (the first sample as `i16` and the step index as `u8`,
//! then a zero byte), then groups of 4 bytes per channel in channel order, each holding eight
//! 4-bit codes, low nibble first. A block of `block_align` bytes holds
//! `(block_align - 4 * channels) * 2 / channels + 1` samples per channel. A code is a sign bit and
//! three magnitude bits: the decoder adds `step/8` plus `step`, `step/2` and `step/4` for each set
//! magnitude bit from the top, then moves the step index by `[-1, -1, -1, -1, 2, 4, 6, 8]`.

use super::{Error, Result, clip16, step_at};

const INDEX_ADJUST: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// The state of one channel's decoder.
#[derive(Clone, Copy)]
struct State {
    predictor: i32,
    index: i32,
}

impl State {
    /// The sample after code `nibble`.
    fn step(&mut self, nibble: u8) -> i16 {
        let step = step_at(self.index);
        let mut diff = step >> 3;
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 8 != 0 {
            diff = -diff;
        }
        self.predictor = clip16(self.predictor + diff);
        self.index = (self.index + INDEX_ADJUST.get(usize::from(nibble & 7)).copied().unwrap_or(0)).clamp(0, 88);
        self.predictor as i16
    }

    /// The code that brings the predictor closest to `sample`, applied to the state.
    fn encode(&mut self, sample: i16) -> u8 {
        let step = step_at(self.index);
        let mut diff = i32::from(sample) - self.predictor;
        let mut code = 0u8;
        if diff < 0 {
            code = 8;
            diff = -diff;
        }
        let mut s = step;
        for bit in [4u8, 2, 1] {
            if diff >= s {
                code |= bit;
                diff -= s;
            }
            s >>= 1;
        }
        self.step(code);
        code
    }
}

/// Samples per channel in a block of `block_align` bytes; `None` when the layout is impossible
/// (a block must hold the headers and whole groups of eight samples, and there are 1 to 8
/// channels).
pub fn samples_per_block(block_align: usize, channels: usize) -> Option<usize> {
    if !(1..=8).contains(&channels) {
        return None;
    }
    let header = 4 * channels;
    let body = block_align.checked_sub(header)?;
    if body == 0 || !body.is_multiple_of(4 * channels) {
        return None;
    }
    let per = (body / channels).checked_mul(2)?.checked_add(1)?;
    // Callers size interleaved buffers from this value; the full count must fit too.
    per.checked_mul(channels)?;
    Some(per)
}

/// Decode one block, appending its samples interleaved by channel to `out`.
pub fn decode_block(block: &[u8], channels: usize, out: &mut Vec<i16>) -> Result<()> {
    let per = samples_per_block(block.len(), channels).ok_or(Error::Malformed("block size does not fit the channel count"))?;
    let mut states = Vec::with_capacity(channels);
    for ch in 0..channels {
        let h = block.get(4 * ch..4 * ch + 4).ok_or(Error::Malformed("truncated block header"))?;
        let index = i32::from(h[2]);
        if index > 88 {
            return Err(Error::Malformed("step index above 88"));
        }
        states.push(State { predictor: i32::from(i16::from_le_bytes([h[0], h[1]])), index });
    }
    let start = out.len();
    out.resize(start + per * channels, 0);
    let samples = out.get_mut(start..).ok_or(Error::Malformed("output"))?;
    for (ch, s) in states.iter().enumerate() {
        if let Some(o) = samples.get_mut(ch) {
            *o = s.predictor as i16;
        }
    }
    let body = block.get(4 * channels..).ok_or(Error::Malformed("truncated block"))?;
    for (g, group) in body.chunks_exact(4 * channels).enumerate() {
        for (ch, bytes) in group.as_chunks::<4>().0.iter().enumerate() {
            let Some(state) = states.get_mut(ch) else { continue };
            for (k, b) in bytes.iter().enumerate() {
                for (half, nibble) in [b & 0xF, b >> 4].into_iter().enumerate() {
                    let t = 1 + g * 8 + k * 2 + half;
                    if let Some(o) = samples.get_mut(t * channels + ch) {
                        *o = state.step(nibble);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Decode a stream of blocks of `block_align` bytes. A final partial block is decoded as far as it
/// holds whole groups.
pub fn decode(data: &[u8], channels: usize, block_align: usize) -> Result<Vec<i16>> {
    samples_per_block(block_align, channels).ok_or(Error::Malformed("block size does not fit the channel count"))?;
    let mut out = Vec::new();
    for block in data.chunks(block_align) {
        if block.len() == block_align {
            decode_block(block, channels, &mut out)?;
        } else {
            let (header, group) = (4 * channels, 4 * channels);
            let whole = block.len().saturating_sub(header) / group * group;
            if whole > 0 {
                decode_block(block.get(..header + whole).unwrap_or(block), channels, &mut out)?;
            }
        }
    }
    Ok(out)
}

/// The starting step index for a channel whose first samples are `x`: the index whose first
/// codes follow `x` most closely.
fn start_index(first: i16, x: &[i16]) -> i32 {
    let mut best = (i64::MAX, 0);
    for index in 0..=88 {
        let mut st = State { predictor: i32::from(first), index };
        let mut err = 0i64;
        for &s in x.iter().take(16) {
            let code = st.encode(s);
            let _ = code;
            err += (i64::from(s) - i64::from(st.predictor)).pow(2);
        }
        if err < best.0 {
            best = (err, index);
        }
    }
    best.1
}

/// Encode one block from exactly `samples_per_block * channels` interleaved samples.
pub fn encode_block(samples: &[i16], channels: usize, block_align: usize) -> Result<Vec<u8>> {
    let per = samples_per_block(block_align, channels).ok_or(Error::InvalidInput("block size does not fit the channel count"))?;
    if samples.len() != per * channels {
        return Err(Error::InvalidInput("a block needs exactly samples_per_block samples per channel"));
    }
    let mut block = vec![0u8; block_align];
    let mut states = Vec::with_capacity(channels);
    for ch in 0..channels {
        let col: Vec<i16> = samples.iter().skip(ch).step_by(channels).copied().collect();
        let first = col.first().copied().unwrap_or(0);
        let st = State { predictor: i32::from(first), index: start_index(first, col.get(1..).unwrap_or(&[])) };
        if let Some(h) = block.get_mut(4 * ch..4 * ch + 4) {
            h[..2].copy_from_slice(&first.to_le_bytes());
            h[2] = st.index as u8;
        }
        states.push(st);
    }
    let groups = (block_align - 4 * channels) / (4 * channels);
    for g in 0..groups {
        for ch in 0..channels {
            let Some(state) = states.get_mut(ch) else { continue };
            for k in 0..4 {
                let mut byte = 0u8;
                for half in 0..2 {
                    let t = 1 + g * 8 + k * 2 + half;
                    let nibble = state.encode(samples.get(t * channels + ch).copied().unwrap_or(0));
                    byte |= nibble << (4 * half);
                }
                if let Some(b) = block.get_mut(4 * channels + g * 4 * channels + ch * 4 + k) {
                    *b = byte;
                }
            }
        }
    }
    Ok(block)
}

/// Encode interleaved samples as blocks of `block_align` bytes. The last block is padded with
/// copies of the last sample, so a decoder returns up to one block more than was given.
pub fn encode(samples: &[i16], channels: usize, block_align: usize) -> Result<Vec<u8>> {
    let per = samples_per_block(block_align, channels).ok_or(Error::InvalidInput("block size does not fit the channel count"))?;
    if !samples.len().is_multiple_of(channels) {
        return Err(Error::InvalidInput("the sample count is not a multiple of the channel count"));
    }
    let mut out = Vec::new();
    for chunk in samples.chunks(per * channels) {
        if chunk.len() == per * channels {
            out.extend(encode_block(chunk, channels, block_align)?);
        } else {
            let mut padded = chunk.to_vec();
            let last: Vec<i16> = chunk.get(chunk.len().saturating_sub(channels)..).unwrap_or(&[]).to_vec();
            while padded.len() < per * channels {
                padded.extend(last.iter().copied().chain(std::iter::repeat(0)).take(channels));
            }
            out.extend(encode_block(&padded, channels, block_align)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        assert_eq!(samples_per_block(256, 1), Some(505));
        assert_eq!(samples_per_block(512, 2), Some(505));
        assert_eq!(samples_per_block(4, 1), None);
        assert_eq!(samples_per_block(100, 0), None);
        assert_eq!(samples_per_block(258, 2), None);
    }

    #[test]
    fn unrepresentable_interleaved_layout_is_rejected() {
        let align = usize::MAX - 7;
        assert_eq!(samples_per_block(align, 2), None);
        assert!(encode(&[], 2, align).is_err());
        assert!(encode_block(&[], 2, align).is_err());
    }

    #[test]
    fn silence_stays_silent() {
        let block = encode_block(&vec![0; 505], 1, 256).unwrap();
        let mut out = Vec::new();
        decode_block(&block, 1, &mut out).unwrap();
        assert!(out.iter().all(|&s| s == 0));
    }
}
