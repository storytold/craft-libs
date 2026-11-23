//! Microsoft ADPCM in WAV blocks (format tag 0x0002).
//!
//! A block starts, for all channels in turn, with one predictor index (a byte, 0 to 6), then one
//! initial step (`delta`, `i16`), then each channel's second-last sample (`sample1`, `i16`) and
//! last sample (`sample2`, `i16`) before the block; the block's first two samples per channel are
//! `sample2` then `sample1`. The rest is 4-bit two's-complement codes, the first channel in the
//! high nibble. A block of `block_align` bytes holds `(block_align - 7 * channels) * 2 / channels
//! + 2` samples per channel. Each sample is predicted as `(sample1 * c1 + sample2 * c2) / 256` (rounding toward zero)
//! from the block's coefficient pair, plus `code * delta`; then `delta` becomes
//! `max(16, delta * a / 256)`, where `a` is the table entry for the code (`ADAPT`).

use super::{Error, Result, clip16};

/// The seven coefficient pairs `(c1, c2)` every Microsoft ADPCM stream uses.
pub const COEFFICIENTS: [(i32, i32); 7] = [(256, 0), (512, -256), (0, 0), (192, 64), (240, 0), (460, -208), (392, -232)];
/// How the step moves with the code's unsigned value (the index is the 4-bit pattern).
/// The largest step. The reference keeps it in 16 bits and no real stream gets near this bound; it
/// keeps every product in range however long a damaged block runs.
const MAX_DELTA: i32 = 1 << 24;
const ADAPT: [i32; 16] = [230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230];

#[derive(Clone, Copy)]
struct State {
    c1: i32,
    c2: i32,
    delta: i32,
    s1: i32,
    s2: i32,
}

impl State {
    fn step(&mut self, nibble: u8) -> i16 {
        let signed = i32::from(nibble) - if nibble & 8 != 0 { 16 } else { 0 };
        let predicted = (self.s1 * self.c1 + self.s2 * self.c2) / 256;
        let sample = clip16(predicted + signed * self.delta);
        self.s2 = self.s1;
        self.s1 = sample;
        let adapt = i64::from(ADAPT.get(usize::from(nibble)).copied().unwrap_or(256));
        let new_delta = (adapt * i64::from(self.delta)) >> 8;
        self.delta = (new_delta.min(i64::from(MAX_DELTA)) as i32).max(16);
        sample as i16
    }

    /// The code that brings the output closest to `sample`, applied to the state.
    fn encode(&mut self, sample: i16) -> u8 {
        let predicted = (self.s1 * self.c1 + self.s2 * self.c2) / 256;
        let err = i32::from(sample) - predicted;
        // round to the nearest multiple of delta
        let q = if err >= 0 { (err + self.delta / 2) / self.delta } else { -((-err + self.delta / 2) / self.delta) };
        let code = (q.clamp(-8, 7) & 0xF) as u8;
        self.step(code);
        code
    }
}

/// Samples per channel in a block of `block_align` bytes; `None` for an impossible layout (1 or 2
/// channels, and room for the header and at least one code byte).
pub fn samples_per_block(block_align: usize, channels: usize) -> Option<usize> {
    if !(1..=2).contains(&channels) {
        return None;
    }
    let body = block_align.checked_sub(7 * channels).filter(|&b| b > 0)?;
    // codes pair up across channels
    let per = (body.checked_mul(2)? / channels).checked_add(2)?;
    // Include the header samples and all channels before callers size their buffers.
    per.checked_mul(channels)?;
    Some(per)
}

/// Decode one block, appending its samples interleaved by channel to `out`.
pub fn decode_block(block: &[u8], channels: usize, out: &mut Vec<i16>) -> Result<()> {
    let per = samples_per_block(block.len(), channels).ok_or(Error::Malformed("block size does not fit the channel count"))?;
    let header = block.get(..7 * channels).ok_or(Error::Malformed("truncated block header"))?;
    let mut states = Vec::with_capacity(channels);
    let word = |i: usize| i32::from(i16::from_le_bytes([header.get(i).copied().unwrap_or(0), header.get(i + 1).copied().unwrap_or(0)]));
    for ch in 0..channels {
        let pred = usize::from(header.get(ch).copied().unwrap_or(0));
        let &(c1, c2) = COEFFICIENTS.get(pred).ok_or(Error::Malformed("predictor index above 6"))?;
        states.push(State { c1, c2, delta: word(channels + 2 * ch), s1: word(3 * channels + 2 * ch), s2: word(5 * channels + 2 * ch) });
    }
    let start = out.len();
    out.resize(start + per * channels, 0);
    let samples = out.get_mut(start..).ok_or(Error::Malformed("output"))?;
    for (ch, s) in states.iter().enumerate() {
        if let Some(a) = samples.get_mut(ch) {
            *a = s.s2 as i16;
        }
        if let Some(b) = samples.get_mut(channels + ch) {
            *b = s.s1 as i16;
        }
    }
    let codes = block.get(7 * channels..).ok_or(Error::Malformed("truncated block"))?;
    let mut t = 2 * channels;
    for byte in codes {
        for nibble in [byte >> 4, byte & 0xF] {
            let ch = t % channels;
            if let (Some(o), Some(state)) = (samples.get_mut(t), states.get_mut(ch)) {
                *o = state.step(nibble);
            }
            t += 1;
        }
    }
    Ok(())
}

/// Decode a stream of blocks of `block_align` bytes. A final partial block is decoded in full if
/// it holds a header and a code byte.
pub fn decode(data: &[u8], channels: usize, block_align: usize) -> Result<Vec<i16>> {
    samples_per_block(block_align, channels).ok_or(Error::Malformed("block size does not fit the channel count"))?;
    let mut out = Vec::new();
    for block in data.chunks(block_align) {
        if block.len() == block_align || samples_per_block(block.len(), channels).is_some() {
            decode_block(block, channels, &mut out)?;
        }
    }
    Ok(out)
}

/// The squared error of encoding `col` (one channel) from state `st`.
fn trial(mut st: State, col: &[i16]) -> i64 {
    let mut err = 0i64;
    for &s in col {
        st.encode(s);
        err += (i64::from(s) - i64::from(st.s1)).pow(2);
    }
    err
}

/// Encode one block from exactly `samples_per_block * channels` interleaved samples. Each channel
/// gets the coefficient pair and initial step that code it with the least error.
pub fn encode_block(samples: &[i16], channels: usize, block_align: usize) -> Result<Vec<u8>> {
    let per = samples_per_block(block_align, channels).ok_or(Error::InvalidInput("block size does not fit the channel count"))?;
    if samples.len() != per * channels {
        return Err(Error::InvalidInput("a block needs exactly samples_per_block samples per channel"));
    }
    let mut block = vec![0u8; block_align];
    let mut states = Vec::with_capacity(channels);
    for ch in 0..channels {
        let col: Vec<i16> = samples.iter().skip(ch).step_by(channels).copied().collect();
        let (x0, x1) = (col.first().copied().unwrap_or(0), col.get(1).copied().unwrap_or(0));
        let rest = col.get(2..).unwrap_or(&[]);
        let mut best: Option<(i64, usize, i32)> = None;
        for (p, &(c1, c2)) in COEFFICIENTS.iter().enumerate() {
            for delta in [16, 64, 256, 1024] {
                let st = State { c1, c2, delta, s1: i32::from(x1), s2: i32::from(x0) };
                let e = trial(st, rest);
                if best.is_none_or(|(b, _, _)| e < b) {
                    best = Some((e, p, delta));
                }
            }
        }
        let (_, p, delta) = best.unwrap_or((0, 0, 16));
        let &(c1, c2) = COEFFICIENTS.get(p).unwrap_or(&(256, 0));
        states.push(State { c1, c2, delta, s1: i32::from(x1), s2: i32::from(x0) });
        let (d, a, b) = (delta as i16, x1, x0);
        let put = |block: &mut Vec<u8>, at: usize, v: i16| {
            if let Some(w) = block.get_mut(at..at + 2) {
                w.copy_from_slice(&v.to_le_bytes());
            }
        };
        if let Some(h) = block.get_mut(ch) {
            *h = p as u8;
        }
        put(&mut block, channels + 2 * ch, d);
        put(&mut block, 3 * channels + 2 * ch, a);
        put(&mut block, 5 * channels + 2 * ch, b);
    }
    let mut t = 2 * channels;
    for byte in block.iter_mut().skip(7 * channels) {
        let mut b = 0u8;
        for half in 0..2 {
            let ch = t % channels;
            let nibble = match states.get_mut(ch) {
                Some(st) => st.encode(samples.get(t).copied().unwrap_or(0)),
                None => 0,
            };
            b |= nibble << (4 * (1 - half));
            t += 1;
        }
        *byte = b;
    }
    Ok(block)
}

/// Encode interleaved samples as blocks of `block_align` bytes. The last block is padded with
/// copies of the last frame, so a decoder returns up to one block more than was given.
pub fn encode(samples: &[i16], channels: usize, block_align: usize) -> Result<Vec<u8>> {
    let per = samples_per_block(block_align, channels).ok_or(Error::InvalidInput("block size does not fit the channel count"))?;
    if !samples.len().is_multiple_of(channels) {
        return Err(Error::InvalidInput("the sample count is not a multiple of the channel count"));
    }
    let mut out = Vec::new();
    for chunk in samples.chunks(per * channels) {
        let mut padded = chunk.to_vec();
        let last: Vec<i16> = chunk.get(chunk.len().saturating_sub(channels)..).unwrap_or(&[]).to_vec();
        while padded.len() < per * channels {
            padded.extend(last.iter().copied().chain(std::iter::repeat(0)).take(channels));
        }
        out.extend(encode_block(&padded, channels, block_align)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        assert_eq!(samples_per_block(256, 1), Some(500));
        assert_eq!(samples_per_block(512, 2), Some(500));
        assert_eq!(samples_per_block(7, 1), None);
        assert_eq!(samples_per_block(100, 3), None);
    }

    #[test]
    fn unrepresentable_layouts_are_rejected() {
        for (align, channels) in [(usize::MAX / 2 + 7, 1), (usize::MAX / 2 + 14, 2)] {
            assert_eq!(samples_per_block(align, channels), None);
            assert!(encode(&[], channels, align).is_err());
            assert!(encode_block(&[], channels, align).is_err());
        }
    }

    #[test]
    fn a_long_run_of_large_codes_cannot_overflow() {
        // a step that starts at its largest 16-bit value and grows with every large code
        let mut block = vec![0x77u8; 7 + 2000];
        block[..7].copy_from_slice(&[0, 255, 127, 0, 0, 0, 0]);
        for b in block.iter_mut().skip(7) {
            *b = 0x88;
        }
        let mut out = Vec::new();
        decode_block(&block, 1, &mut out).unwrap();
        assert_eq!(out.len(), 2000 * 2 + 2);
    }

    #[test]
    fn bad_predictor_index_is_an_error() {
        let mut block = vec![0u8; 64];
        block[0] = 7;
        assert!(decode_block(&block, 1, &mut Vec::new()).is_err());
    }
}
