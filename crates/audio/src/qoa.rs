//! QOA ("Quite OK Audio"): a decoder (feature `qoa-decode`) and an encoder (feature `qoa-encode`)
//! for whole `.qoa` files, 16-bit, 1 to 8 channels.
//!
//! Written from the published QOA specification (qoaformat.org). A file is an 8-byte header (the
//! magic `qoaf` and the number of samples per channel) followed by frames of up to 5120 samples
//! per channel. Each frame starts with its channel count, sample rate, sample count and size, then
//! the state of one 4-tap sign-sign LMS predictor per channel, then 64-bit slices of 20 samples:
//! a 4-bit scale factor and twenty 3-bit quantised residuals, the channels interleaved slice by
//! slice.
//!
//! Streaming files (a sample count of 0 in the header) are not supported.
//! Samples are `i16`, one `Vec` per channel.

#[cfg(feature = "qoa-decode")]
const MAX_FRAME_SAMPLES: usize = 5120;
const SLICE_SAMPLES: usize = 20;
const LMS_LEN: usize = 4;
const MAGIC: &[u8; 4] = b"qoaf";
/// The most channels a file may have.
pub const MAX_CHANNELS: usize = 8;

/// `dequant[scale factor][quantised residual]`: `round(round((sf + 1)^2.75) * [0.75, -0.75, 2.5,
/// -2.5, 4.5, -4.5, 7, -7])`.
const DEQUANT: [[i32; 8]; 16] = [
    [1, -1, 3, -3, 5, -5, 7, -7],
    [5, -5, 18, -18, 32, -32, 49, -49],
    [16, -16, 53, -53, 95, -95, 147, -147],
    [34, -34, 113, -113, 203, -203, 315, -315],
    [63, -63, 210, -210, 378, -378, 588, -588],
    [104, -104, 345, -345, 621, -621, 966, -966],
    [158, -158, 528, -528, 950, -950, 1477, -1477],
    [228, -228, 760, -760, 1368, -1368, 2128, -2128],
    [316, -316, 1053, -1053, 1895, -1895, 2947, -2947],
    [422, -422, 1405, -1405, 2529, -2529, 3934, -3934],
    [548, -548, 1828, -1828, 3290, -3290, 5117, -5117],
    [696, -696, 2320, -2320, 4176, -4176, 6496, -6496],
    [868, -868, 2893, -2893, 5207, -5207, 8099, -8099],
    [1064, -1064, 3548, -3548, 6386, -6386, 9933, -9933],
    [1286, -1286, 4288, -4288, 7718, -7718, 12005, -12005],
    [1536, -1536, 5120, -5120, 9216, -9216, 14336, -14336],
];

/// What went wrong reading or writing a QOA file.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The input is not a QOA file, or is damaged or truncated.
    #[error("malformed QOA: {0}")]
    Malformed(&'static str),
    /// A valid file this module does not handle (a streaming file).
    #[error("unsupported QOA: {0}")]
    Unsupported(&'static str),
    /// The samples or settings given to the encoder cannot be written as QOA.
    #[error("QOA cannot encode: {0}")]
    InvalidInput(&'static str),
}

/// A decoded file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audio {
    /// Sample rate in Hz (1 to 16 777 215).
    pub sample_rate: u32,
    /// One `Vec` of samples per channel, all the same length.
    pub channels: Vec<Vec<i16>>,
}

/// The predictor: the last four samples and four weights.
#[derive(Clone, Copy)]
struct Lms {
    history: [i32; LMS_LEN],
    weights: [i32; LMS_LEN],
}

impl Lms {
    fn new() -> Self {
        Lms { history: [0; LMS_LEN], weights: [0, 0, -(1 << 13), 1 << 14] }
    }

    fn predict(&self) -> i32 {
        let mut p = 0i32;
        for i in 0..LMS_LEN {
            p = p.wrapping_add(self.weights[i].wrapping_mul(self.history[i]));
        }
        p >> 13
    }

    fn update(&mut self, sample: i32, residual: i32) {
        let delta = residual >> 4;
        for i in 0..LMS_LEN {
            self.weights[i] += if self.history[i] < 0 { -delta } else { delta };
        }
        self.history.copy_within(1.., 0);
        self.history[LMS_LEN - 1] = sample;
    }
}

fn clamp16(v: i32) -> i32 {
    v.clamp(-32768, 32767)
}

#[cfg(feature = "qoa-decode")]
mod decode {
    use super::{Audio, DEQUANT, Error, LMS_LEN, Lms, MAGIC, MAX_CHANNELS, MAX_FRAME_SAMPLES, SLICE_SAMPLES, clamp16};

    fn be16(b: &[u8], i: usize) -> u16 {
        u16::from_be_bytes([b[i], b[i + 1]])
    }

    /// Decode a whole `.qoa` file.
    ///
    /// The declared length is checked against the input before anything is allocated: a slice of
    /// 8 bytes holds at most 20 samples, so a file cannot declare more samples than that.
    pub fn decode(bytes: &[u8]) -> Result<Audio, Error> {
        if bytes.len() < 8 || &bytes[..4] != MAGIC {
            return Err(Error::Malformed("missing the qoaf magic"));
        }
        let total = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        if total == 0 {
            return Err(Error::Unsupported("streaming files (a sample count of 0)"));
        }
        if bytes.len() < 16 {
            return Err(Error::Malformed("truncated frame header"));
        }
        let channels = usize::from(bytes[8]);
        let sample_rate = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        if channels == 0 || channels > MAX_CHANNELS || sample_rate == 0 {
            return Err(Error::Malformed("bad channel count or sample rate"));
        }
        // 8 bytes (20 samples) per channel at the very least, plus the frame headers.
        let room = (bytes.len() - 8) / (8 * channels) * SLICE_SAMPLES;
        if total > room {
            return Err(Error::Malformed("the file is too short for its declared length"));
        }
        let mut out: Vec<Vec<i16>> = Vec::new();
        for _ in 0..channels {
            let mut v = Vec::new();
            v.try_reserve_exact(total).map_err(|_| Error::Malformed("too many samples"))?;
            out.push(v);
        }
        let mut pos = 8;
        let mut done = 0usize;
        while done < total {
            let head = bytes.get(pos..pos + 8).ok_or(Error::Malformed("truncated frame header"))?;
            let rate = u32::from_be_bytes([0, head[1], head[2], head[3]]);
            if usize::from(head[0]) != channels || rate != sample_rate {
                return Err(Error::Malformed("the channel count or sample rate changes between frames"));
            }
            let samples = usize::from(be16(head, 4));
            let size = usize::from(be16(head, 6));
            let slices = samples.div_ceil(SLICE_SAMPLES);
            if samples == 0 || samples > MAX_FRAME_SAMPLES || samples > total - done {
                return Err(Error::Malformed("bad frame sample count"));
            }
            if size != 8 + 16 * channels + 8 * slices * channels {
                return Err(Error::Malformed("frame size does not match its sample count"));
            }
            let frame = bytes.get(pos..pos + size).ok_or(Error::Malformed("truncated frame"))?;
            let mut lms = [Lms::new(); MAX_CHANNELS];
            for (c, state) in lms.iter_mut().take(channels).enumerate() {
                let at = 8 + c * 16;
                for i in 0..LMS_LEN {
                    state.history[i] = i32::from(be16(frame, at + 2 * i) as i16);
                    state.weights[i] = i32::from(be16(frame, at + 8 + 2 * i) as i16);
                }
            }
            let mut at = 8 + 16 * channels;
            for s in 0..slices {
                let n = (samples - s * SLICE_SAMPLES).min(SLICE_SAMPLES);
                for (c, state) in lms.iter_mut().take(channels).enumerate() {
                    let mut slice = u64::from_be_bytes([
                        frame[at],
                        frame[at + 1],
                        frame[at + 2],
                        frame[at + 3],
                        frame[at + 4],
                        frame[at + 5],
                        frame[at + 6],
                        frame[at + 7],
                    ]);
                    at += 8;
                    let table = &DEQUANT[(slice >> 60) as usize & 15];
                    for _ in 0..n {
                        let dequantised = table[(slice >> 57) as usize & 7];
                        slice <<= 3;
                        let sample = clamp16(state.predict() + dequantised);
                        out[c].push(sample as i16);
                        state.update(sample, dequantised);
                    }
                }
            }
            pos += size;
            done += samples;
        }
        Ok(Audio { sample_rate, channels: out })
    }
}
#[cfg(feature = "qoa-decode")]
pub use decode::decode;

#[cfg(feature = "qoa-encode")]
mod encode {
    use super::{DEQUANT, Error, Lms, MAGIC, MAX_CHANNELS, SLICE_SAMPLES, clamp16};

    /// The quantised residual (0..8) whose dequantised value is nearest `residual`.
    fn quantise(table: &[i32; 8], residual: i32) -> usize {
        let mut best = 0;
        let mut best_err = i64::MAX;
        for (q, &d) in table.iter().enumerate() {
            let e = i64::from(residual - d).pow(2);
            if e < best_err {
                best = q;
                best_err = e;
            }
        }
        best
    }

    /// Encode one slice of one channel with every scale factor and keep the one with the least
    /// squared error. Returns the 64-bit slice; `lms` is advanced to the chosen result.
    fn encode_slice(lms: &mut Lms, samples: &[i16]) -> u64 {
        let mut best: Option<(i64, u64, Lms)> = None;
        for (sf, table) in DEQUANT.iter().enumerate() {
            let mut trial = *lms;
            let mut slice = sf as u64;
            let mut err = 0i64;
            for k in 0..SLICE_SAMPLES {
                let mut q = 0;
                if let Some(&s) = samples.get(k) {
                    let predicted = trial.predict();
                    q = quantise(table, i32::from(s) - predicted);
                    let sample = clamp16(predicted + table[q]);
                    err += i64::from(i32::from(s) - sample).pow(2);
                    trial.update(sample, table[q]);
                }
                slice = slice << 3 | q as u64;
            }
            // 4 + 60 bits are in `slice`; the scale factor belongs on top.
            let slice = slice & ((1 << 60) - 1) | (sf as u64) << 60;
            if best.as_ref().is_none_or(|b| err < b.0) {
                best = Some((err, slice, trial));
            }
        }
        match best {
            Some((_, slice, state)) => {
                *lms = state;
                slice
            }
            None => 0,
        }
    }

    /// Encode `channels` (one `Vec` of samples per channel, all the same length) as a `.qoa` file.
    pub fn encode(sample_rate: u32, channels: &[Vec<i16>]) -> Result<Vec<u8>, Error> {
        let n = channels.len();
        if n == 0 || n > MAX_CHANNELS {
            return Err(Error::InvalidInput("QOA stores 1 to 8 channels"));
        }
        if sample_rate == 0 || sample_rate > 0x00ff_ffff {
            return Err(Error::InvalidInput("the sample rate must be 1 to 16777215 Hz"));
        }
        let total = channels[0].len();
        if channels.iter().any(|c| c.len() != total) {
            return Err(Error::InvalidInput("the channels differ in length"));
        }
        let total32 = u32::try_from(total).map_err(|_| Error::InvalidInput("too many samples"))?;
        if total == 0 {
            return Err(Error::InvalidInput("QOA cannot store an empty file"));
        }
        let mut out = Vec::with_capacity(8 + total * n / 2);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&total32.to_be_bytes());
        let mut lms = [Lms::new(); MAX_CHANNELS];
        let mut start = 0;
        while start < total {
            let len = (total - start).min(5120);
            let slices = len.div_ceil(SLICE_SAMPLES);
            let size = 8 + 16 * n + 8 * slices * n;
            out.push(n as u8);
            out.extend_from_slice(&sample_rate.to_be_bytes()[1..]);
            out.extend_from_slice(&(len as u16).to_be_bytes());
            out.extend_from_slice(&(size as u16).to_be_bytes());
            // The state a frame starts from is written as 16-bit values, so the predictor
            // continues from exactly what a decoder reads.
            for state in lms.iter_mut().take(n) {
                for v in state.history.iter_mut().chain(state.weights.iter_mut()) {
                    *v = i32::from(*v as i16);
                    out.extend_from_slice(&(*v as i16).to_be_bytes());
                }
            }
            for s in 0..slices {
                let from = start + s * SLICE_SAMPLES;
                let to = (from + SLICE_SAMPLES).min(start + len);
                for (c, state) in lms.iter_mut().take(n).enumerate() {
                    out.extend_from_slice(&encode_slice(state, &channels[c][from..to]).to_be_bytes());
                }
            }
            start += len;
        }
        Ok(out)
    }
}
#[cfg(feature = "qoa-encode")]
pub use encode::encode;

#[cfg(all(test, feature = "qoa-decode", feature = "qoa-encode"))]
mod tests {
    use super::*;

    fn tone(frames: usize, ch: usize) -> Vec<Vec<i16>> {
        (0..ch)
            .map(|c| (0..frames).map(|i| (((i as f64) * (0.03 + 0.01 * c as f64)).sin() * 12000.0 + ((i * 7919) % 200) as f64) as i16).collect())
            .collect()
    }

    fn snr(a: &[i16], b: &[i16]) -> f64 {
        let (mut s, mut e) = (0f64, 0f64);
        for (&x, &y) in a.iter().zip(b) {
            s += f64::from(x).powi(2);
            e += (f64::from(x) - f64::from(y)).powi(2);
        }
        10.0 * (s / e.max(1.0)).log10()
    }

    #[test]
    fn dequant_table_matches_its_formula() {
        for (sf, row) in DEQUANT.iter().enumerate() {
            let scale = ((sf as f64 + 1.0).powf(2.75)).round();
            for (q, base) in [0.75, -0.75, 2.5, -2.5, 4.5, -4.5, 7.0, -7.0].into_iter().enumerate() {
                assert_eq!(f64::from(row[q]), (scale * base).round(), "sf {sf} q {q}");
            }
        }
    }

    #[test]
    fn round_trips_with_a_good_signal_to_noise_ratio() {
        for (frames, ch) in [(1, 1), (19, 2), (20, 1), (21, 2), (5120, 1), (5121, 2), (12_345, 8)] {
            let src = tone(frames, ch);
            let bytes = encode(44_100, &src).unwrap();
            let back = decode(&bytes).unwrap();
            assert_eq!((back.sample_rate, back.channels.len(), back.channels[0].len()), (44_100, ch, frames));
            if frames > 1000 {
                for (c, (a, b)) in src.iter().zip(&back.channels).enumerate() {
                    assert!(snr(a, b) > 25.0, "{frames}x{ch} channel {c}");
                }
            }
        }
    }

    #[test]
    fn silence_and_full_scale_extremes_survive() {
        let silent = vec![vec![0i16; 3000]];
        // No residual value is zero, so silence decodes to a tiny residue, never exactly 0.
        let quiet = decode(&encode(8000, &silent).unwrap()).unwrap();
        assert!(quiet.channels[0].iter().all(|&s| s.abs() <= 2), "silence stays within 2 LSB");
        let square: Vec<i16> = (0..4000).map(|i| if (i / 50) % 2 == 0 { i16::MAX } else { i16::MIN }).collect();
        let back = decode(&encode(48_000, std::slice::from_ref(&square)).unwrap()).unwrap();
        assert_eq!(back.channels[0].len(), square.len());
    }

    #[test]
    fn the_file_has_the_documented_layout() {
        let bytes = encode(48_000, &tone(100, 2)).unwrap();
        assert_eq!(&bytes[..4], b"qoaf");
        assert_eq!(u32::from_be_bytes(bytes[4..8].try_into().unwrap()), 100);
        assert_eq!(bytes[8], 2);
        assert_eq!(&bytes[9..12], &48_000u32.to_be_bytes()[1..]);
        assert_eq!(u16::from_be_bytes([bytes[12], bytes[13]]), 100);
        // 8 header + 2 channels * 16 state + 5 slices * 2 channels * 8
        assert_eq!(usize::from(u16::from_be_bytes([bytes[14], bytes[15]])), 8 + 32 + 80);
        assert_eq!(bytes.len(), 8 + 8 + 32 + 80);
        // The first frame starts from the specified predictor state.
        assert_eq!(&bytes[16 + 8..16 + 16], &[0, 0, 0, 0, 0xe0, 0, 0x40, 0]);
    }

    #[test]
    fn invalid_input_to_the_encoder_is_refused() {
        assert!(encode(44_100, &[]).is_err());
        assert!(encode(0, &[vec![0; 10]]).is_err());
        assert!(encode(1 << 24, &[vec![0; 10]]).is_err());
        assert!(encode(44_100, &[vec![0; 10], vec![0; 9]]).is_err());
        assert!(encode(44_100, &[vec![]]).is_err());
        assert!(encode(44_100, &vec![vec![0; 10]; 9]).is_err());
    }

    #[test]
    fn hostile_input_is_an_error_not_a_panic() {
        let good = encode(44_100, &tone(6000, 2)).unwrap();
        for len in 0..good.len().min(600) {
            let _ = decode(&good[..len]);
        }
        assert!(decode(&good[..good.len() - 1]).is_err());
        // A header that declares more samples than the bytes can hold.
        let mut huge = good.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&huge).is_err());
        // A streaming file, and a frame whose size field lies.
        let mut streaming = good.clone();
        streaming[4..8].fill(0);
        assert_eq!(decode(&streaming), Err(Error::Unsupported("streaming files (a sample count of 0)")));
        let mut lying = good.clone();
        lying[14] ^= 1;
        assert!(decode(&lying).is_err());
        let mut zero_rate = good.clone();
        zero_rate[9..12].fill(0);
        assert!(decode(&zero_rate).is_err());
        let mut changed = good;
        let second = 8 + usize::from(u16::from_be_bytes([changed[14], changed[15]]));
        changed[second] = 1;
        assert!(decode(&changed).is_err());
    }
}
