//! DSD-to-PCM decimation filter.
//!
//! Direct Stream Digital (DSD) uses 1-bit Sigma-Delta modulation sampled at high rates
//! (2,822,400 Hz for DSD64, 5,644,800 Hz for DSD128, etc.). Decimation to multi-bit PCM
//! involves low-pass filtering to attenuate high-frequency quantization noise followed
//! by downsampling.
//!
//! This module provides a 96-tap symmetric FIR decimation filter that downsamples 1-bit DSD
//! to multi-bit PCM at 1/8th of the DSD sampling rate (e.g. 352,800 Hz for DSD64). Further
//! integer decimation (to 176.4 kHz, 88.2 kHz, or 44.1 kHz) is supported via subsequent
//! downsampling stages.

use super::{BitOrder, Error, MAX_DECODE_BYTES};

/// 96-tap symmetric FIR decimation filter coefficients for decimation by 8.
///
/// Output is normalized so that alternating silence bits (0x69 or 0xAA) decode near 0.0,
/// and full-scale runs of 1s or 0s decode near +1.0 and -1.0.
#[allow(clippy::excessive_precision)]
pub const TAPS_96: [f32; 96] = [
    2.9802322388e-08_f32,
    1.1920928955e-07_f32,
    3.5762786865e-07_f32,
    7.4505805969e-07_f32,
    1.3113021851e-06_f32,
    1.9371509552e-06_f32,
    2.1755695343e-06_f32,
    1.2516975403e-06_f32,
    -1.9967556000e-06_f32,
    -9.1493129730e-06_f32,
    -2.1725893021e-05_f32,
    -4.0739774704e-05_f32,
    -6.5773725510e-05_f32,
    -9.3966722488e-05_f32,
    -1.1900067329e-04_f32,
    -1.3050436974e-04_f32,
    -1.1405348778e-04_f32,
    -5.2779912949e-05_f32,
    7.0035457611e-05_f32,
    2.6643276215e-04_f32,
    5.3817033768e-04_f32,
    8.7043642998e-04_f32,
    1.2266933918e-03_f32,
    1.5456080437e-03_f32,
    1.7420351505e-03_f32,
    1.7137229443e-03_f32,
    1.3538300991e-03_f32,
    5.7008862495e-04_f32,
    -6.9221854210e-04_f32,
    -2.4250447750e-03_f32,
    -4.5351684093e-03_f32,
    -6.8288743496e-03_f32,
    -9.0079009533e-03_f32,
    -1.0681360960e-02_f32,
    -1.1394292116e-02_f32,
    -1.0672420263e-02_f32,
    -8.0802738667e-03_f32,
    -3.2847225666e-03_f32,
    3.8830339909e-03_f32,
    1.3377487659e-02_f32,
    2.4909228086e-02_f32,
    3.7942945957e-02_f32,
    5.1726281643e-02_f32,
    6.5348774195e-02_f32,
    7.7825516462e-02_f32,
    8.8196486235e-02_f32,
    9.5628470182e-02_f32,
    9.9507331848e-02_f32,
    9.9507331848e-02_f32,
    9.5628470182e-02_f32,
    8.8196486235e-02_f32,
    7.7825516462e-02_f32,
    6.5348774195e-02_f32,
    5.1726281643e-02_f32,
    3.7942945957e-02_f32,
    2.4909228086e-02_f32,
    1.3377487659e-02_f32,
    3.8830339909e-03_f32,
    -3.2847225666e-03_f32,
    -8.0802738667e-03_f32,
    -1.0672420263e-02_f32,
    -1.1394292116e-02_f32,
    -1.0681360960e-02_f32,
    -9.0079009533e-03_f32,
    -6.8288743496e-03_f32,
    -4.5351684093e-03_f32,
    -2.4250447750e-03_f32,
    -6.9221854210e-04_f32,
    5.7008862495e-04_f32,
    1.3538300991e-03_f32,
    1.7137229443e-03_f32,
    1.7420351505e-03_f32,
    1.5456080437e-03_f32,
    1.2266933918e-03_f32,
    8.7043642998e-04_f32,
    5.3817033768e-04_f32,
    2.6643276215e-04_f32,
    7.0035457611e-05_f32,
    -5.2779912949e-05_f32,
    -1.1405348778e-04_f32,
    -1.3050436974e-04_f32,
    -1.1900067329e-04_f32,
    -9.3966722488e-05_f32,
    -6.5773725510e-05_f32,
    -4.0739774704e-05_f32,
    -2.1725893021e-05_f32,
    -9.1493129730e-06_f32,
    -1.9967556000e-06_f32,
    1.2516975403e-06_f32,
    2.1755695343e-06_f32,
    1.9371509552e-06_f32,
    1.3113021851e-06_f32,
    7.4505805969e-07_f32,
    3.5762786865e-07_f32,
    1.1920928955e-07_f32,
    2.9802322388e-08_f32,
];

/// The number of filter history bytes (96 taps / 8 bits per byte = 12 bytes).
pub const FILTER_BYTES: usize = 12;

/// Standard DSD silence pattern byte for LSB-first ordering (Sony DSF).
pub const SILENCE_LSBF: u8 = 0x69;

/// Standard DSD silence pattern byte for MSB-first ordering (Philips DSDIFF).
pub const SILENCE_MSBF: u8 = 0xAA;

/// Precomputes lookup table entries for byte position `k` (0..12) and byte value `v` (0..256).
///
/// In LSB-first order, bit `b` of input byte `k` corresponds to tap `k * 8 + b`.
#[inline]
fn byte_lut(k: usize, byte_val: u8) -> f32 {
    let mut sum = 0.0_f32;
    let base_tap = k * 8;
    for b in 0..8 {
        if ((byte_val >> b) & 1) != 0
            && let Some(tap) = TAPS_96.get(base_tap + b)
        {
            sum += 2.0 * *tap;
        }
    }
    sum
}

/// Decimates 1-bit DSD audio bytes for a single channel to `f32` PCM.
///
/// The default decimation factor is 8 (1 output float per input byte).
/// When `factor` is 16, 32, or 64, the 1/8 output is further downsampled
/// by 2, 4, or 8 respectively.
pub fn decimate_channel(channel_bytes: &[u8], bit_order: BitOrder, factor: u32) -> Result<Vec<f32>, Error> {
    if !matches!(factor, 8 | 16 | 32 | 64) {
        return Err(Error::Unsupported("decimation factor must be 8, 16, 32, or 64"));
    }

    let num_bytes = channel_bytes.len();
    if num_bytes.saturating_mul(std::mem::size_of::<f32>()) > MAX_DECODE_BYTES {
        return Err(Error::MemoryLimitExceeded);
    }

    // Build the 12 x 256 lookup table for fast convolution.
    // 12 * 256 * 4 bytes = 12 KB, sits comfortably in L1 cache.
    let mut lut = [[0.0_f32; 256]; FILTER_BYTES];
    for (k, lut_k) in lut.iter_mut().enumerate() {
        for (v, cell) in lut_k.iter_mut().enumerate() {
            *cell = byte_lut(k, v as u8);
        }
    }

    let mut stage1 = Vec::new();
    stage1.try_reserve_exact(num_bytes).map_err(|_| Error::MemoryLimitExceeded)?;

    // History buffer holds the most recent 12 bytes in LSB-first order.
    // Pre-filled with silence (0x69).
    let mut history = [SILENCE_LSBF; FILTER_BYTES];

    for &raw_byte in channel_bytes {
        let byte = match bit_order {
            BitOrder::LsbFirst => raw_byte,
            BitOrder::MsbFirst => raw_byte.reverse_bits(),
        };

        // Shift history left by 1 and push current byte to the end.
        history.copy_within(1.., 0);
        if let Some(last) = history.last_mut() {
            *last = byte;
        }

        let mut sample = -1.0_f32;
        for (k, &hist_byte) in history.iter().enumerate() {
            if let Some(lut_k) = lut.get(k)
                && let Some(&contrib) = lut_k.get(hist_byte as usize)
            {
                sample += contrib;
            }
        }

        stage1.push(sample);
    }

    if factor == 8 {
        return Ok(stage1);
    }

    // Secondary downsampling by ratio = factor / 8 (2, 4, or 8).
    let ratio = (factor / 8) as usize;
    let out_len = stage1.len() / ratio;
    let mut out = Vec::new();
    out.try_reserve_exact(out_len).map_err(|_| Error::MemoryLimitExceeded)?;

    for chunk in stage1.chunks_exact(ratio) {
        let sum: f32 = chunk.iter().sum();
        out.push(sum / ratio as f32);
    }

    Ok(out)
}
