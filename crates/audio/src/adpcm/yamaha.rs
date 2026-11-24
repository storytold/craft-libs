//! Yamaha ADPCM (the ADPCM-B of the YM2608 and the AICA): a raw stream of 4-bit codes, no header.
//!
//! Each channel starts at predictor 0 and step 127. A code is a sign bit and three magnitude bits;
//! the sample moves by `step * [1, 3, 5, 7, 9, 11, 13, 15][magnitude] / 8` (negative when the sign
//! is set) and the step is multiplied by `[230, 230, 230, 230, 307, 409, 512, 614][magnitude] / 256`,
//! kept within 127 and 24576. One channel: the low nibble of a byte is the first sample. Two
//! channels: the low nibble is the left channel and the high nibble the right one of the same
//! frame.

use super::{Error, Result, clip16};

const SCALE: [i32; 8] = [230, 230, 230, 230, 307, 409, 512, 614];

#[derive(Clone, Copy)]
struct State {
    predictor: i32,
    step: i32,
}

impl State {
    fn new() -> State {
        State { predictor: 0, step: 127 }
    }

    fn apply(&mut self, nibble: u8) -> i16 {
        let m = usize::from(nibble & 7);
        let diff = self.step * (2 * m as i32 + 1) / 8;
        self.predictor = clip16(self.predictor + if nibble & 8 != 0 { -diff } else { diff });
        self.step = ((self.step * SCALE.get(m).copied().unwrap_or(256)) >> 8).clamp(127, 24_576);
        self.predictor as i16
    }

    fn encode(&mut self, sample: i16) -> u8 {
        let err = i32::from(sample) - self.predictor;
        let m = (err.abs() * 8 / self.step / 2).clamp(0, 7);
        // the magnitude whose reconstruction (2m + 1) * step / 8 is nearest
        let mut best = (i32::MAX, 0u8);
        for cand in [m - 1, m, m + 1] {
            let c = cand.clamp(0, 7);
            let recon = self.step * (2 * c + 1) / 8;
            let e = (err.abs() - recon).abs();
            if e < best.0 {
                best = (e, c as u8);
            }
        }
        let nibble = best.1 | if err < 0 { 8 } else { 0 };
        self.apply(nibble);
        nibble
    }
}

/// Decode a stream of `channels` (1 or 2) interleaved channels.
pub fn decode(data: &[u8], channels: usize) -> Result<Vec<i16>> {
    if !(1..=2).contains(&channels) {
        return Err(Error::Malformed("Yamaha ADPCM has one or two channels"));
    }
    let mut states = vec![State::new(); channels];
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut t = 0usize;
    for byte in data {
        for nibble in [byte & 0xF, byte >> 4] {
            if let Some(st) = states.get_mut(t % channels) {
                out.push(st.apply(nibble));
            }
            t += 1;
        }
    }
    Ok(out)
}

/// Encode interleaved samples. With two channels the frame count must be whole bytes, which it
/// always is; with one channel an odd count is padded with a copy of the last sample.
pub fn encode(samples: &[i16], channels: usize) -> Result<Vec<u8>> {
    if !(1..=2).contains(&channels) {
        return Err(Error::InvalidInput("Yamaha ADPCM has one or two channels"));
    }
    if !samples.len().is_multiple_of(channels) {
        return Err(Error::InvalidInput("the sample count is not a multiple of the channel count"));
    }
    let mut states = vec![State::new(); channels];
    let mut out = Vec::with_capacity(samples.len().div_ceil(2));
    let mut t = 0usize;
    let mut padded = samples.to_vec();
    if padded.len() % 2 == 1 {
        padded.push(padded.last().copied().unwrap_or(0));
    }
    for pair in padded.chunks(2) {
        let mut byte = 0u8;
        for (half, &s) in pair.iter().enumerate() {
            let code = match states.get_mut(t % channels) {
                Some(st) => st.encode(s),
                None => 0,
            };
            byte |= code << (4 * half);
            t += 1;
        }
        out.push(byte);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_count_is_checked() {
        assert!(decode(&[0], 0).is_err());
        assert!(decode(&[0], 3).is_err());
        assert!(encode(&[0], 3).is_err());
        assert!(encode(&[0, 0, 0], 2).is_err());
    }
}
