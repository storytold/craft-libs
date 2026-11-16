//! Interleaved PCM packet decoding into caller-owned planar channels.

/// The sample representation in an interleaved PCM packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// Bits per sample.
    pub bits: u16,
    /// IEEE floating-point samples.
    pub float: bool,
    /// Big-endian sample bytes.
    pub big_endian: bool,
    /// Signed 8-bit integers (wider integer PCM is signed).
    pub signed: bool,
}

/// Append complete interleaved frames to caller-owned planar channels. Empty channel slices and
/// zero-bit formats append nothing. Incomplete frames are ignored; unsupported widths produce
/// silent samples, matching the container adapters that use this API.
pub fn decode_into(data: &[u8], cfg: Format, out: &mut [Vec<f32>]) {
    let ch = out.len();
    if cfg.bits == 0 || ch == 0 {
        return;
    }
    let bps = usize::from(cfg.bits).div_ceil(8);
    let Some(frame_bytes) = bps.checked_mul(ch).filter(|&n| n > 0) else { return };
    let n = data.len() / frame_bytes;
    for channel in out.iter_mut() {
        channel.reserve(n);
    }
    // `n` is whole frames within `data`, so every sample slice below is in bounds.
    for i in 0..n {
        for (c, o) in out.iter_mut().enumerate() {
            let off = (i * ch + c) * bps;
            let b = &data[off..off + bps];
            let v = match (cfg.bits, cfg.float, cfg.big_endian) {
                (32, true, false) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                (32, true, true) => f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                (64, true, false) => f64::from_le_bytes(b.try_into().unwrap_or([0; 8])) as f32,
                (64, true, true) => f64::from_be_bytes(b.try_into().unwrap_or([0; 8])) as f32,
                (8, _, _) => {
                    if cfg.signed {
                        b[0] as i8 as f32 / 128.0
                    } else {
                        (b[0] as f32 - 128.0) / 128.0
                    }
                }
                (16, _, false) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                (16, _, true) => i16::from_be_bytes([b[0], b[1]]) as f32 / 32768.0,
                (24, _, false) => (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0,
                (24, _, true) => (i32::from_be_bytes([b[0], b[1], b[2], 0]) >> 8) as f32 / 8_388_608.0,
                (32, false, false) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
                (32, false, true) => i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
                _ => 0.0,
            };
            o.push(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_endian_and_interleaving_without_touching_partial_frames() {
        let mut channels = vec![Vec::new(), Vec::new()];
        let fmt = Format { bits: 16, float: false, big_endian: true, signed: true };
        decode_into(&[0x40, 0, 0xc0, 0, 0xff], fmt, &mut channels);
        assert_eq!(channels, [vec![0.5], vec![-0.5]]);
        decode_into(&[0; 8], Format { bits: 0, ..fmt }, &mut channels);
        assert_eq!(channels, [vec![0.5], vec![-0.5]]);
    }
    #[test]
    fn decodes_sample_representations_and_ignores_invalid_inputs() {
        let base = Format { bits: 8, float: false, big_endian: false, signed: false };
        let cases: &[(Format, &[u8], f32)] = &[
            (base, &[192], 0.5),
            (Format { signed: true, ..base }, &[192], -0.5),
            (Format { bits: 16, ..base }, &[0, 64], 0.5),
            (Format { bits: 24, ..base }, &[0, 0, 64], 0.5),
            (Format { bits: 24, big_endian: true, ..base }, &[0xc0, 0, 0], -0.5),
            (Format { bits: 32, ..base }, &[0, 0, 0, 64], 0.5),
            (Format { bits: 32, big_endian: true, ..base }, &[0xc0, 0, 0, 0], -0.5),
            (Format { bits: 32, float: true, ..base }, &0.5_f32.to_le_bytes(), 0.5),
            (Format { bits: 32, float: true, big_endian: true, ..base }, &(-0.5_f32).to_be_bytes(), -0.5),
            (Format { bits: 64, float: true, ..base }, &0.5_f64.to_le_bytes(), 0.5),
            (Format { bits: 64, float: true, big_endian: true, ..base }, &(-0.5_f64).to_be_bytes(), -0.5),
            (Format { bits: 12, ..base }, &[0xff, 0xff], 0.0),
        ];
        for &(format, bytes, expected) in cases {
            let mut out = [vec![1.0]];
            decode_into(bytes, format, &mut out);
            assert_eq!(out[0], [1.0, expected], "{format:?}");
        }
        decode_into(&[1], base, &mut []);
    }
}
