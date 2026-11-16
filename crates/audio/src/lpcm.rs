//! DVD-Video and Blu-ray LPCM packet grammar and sample decoding.

/// Blu-ray / AVCHD LPCM packet header (4 bytes before the samples).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlurayLpcm {
    /// Sample rate in hertz.
    pub sample_rate: u32,
    /// Number of decoded channels.
    pub channels: usize,
    /// Significant bits per sample.
    pub bits: u32,
}

/// Channels of each channel_assignment code (mono, stereo, 3/0, 2/1, 3/1, 2/2, 3/2, 3/2+LFE,
/// 3/4, 3/4+LFE).
const BD_LPCM_CHANNELS: [usize; 12] = [0, 1, 0, 2, 3, 3, 4, 4, 5, 6, 7, 8];

/// Parse and validate the four-byte Blu-ray LPCM header.
pub fn bluray_lpcm_header(p: &[u8]) -> Option<BlurayLpcm> {
    if p.len() < 4 {
        return None;
    }
    let channels = *BD_LPCM_CHANNELS.get((p[2] >> 4) as usize).filter(|&&c| c > 0)?;
    let sample_rate = match p[2] & 15 {
        1 => 48_000,
        4 => 96_000,
        5 => 192_000,
        _ => return None,
    };
    let bits = match p[3] >> 6 {
        1 => 16,
        2 => 20,
        3 => 24,
        _ => return None,
    };
    Some(BlurayLpcm { sample_rate, channels, bits })
}

/// Decode one Blu-ray LPCM packet (big-endian; channels padded to an even count; 20-bit
/// samples are stored in 24 bits).
pub fn decode_bluray_lpcm(p: &[u8]) -> Option<Vec<Vec<f32>>> {
    let h = bluray_lpcm_header(p)?;
    let coded = (h.channels + 1) & !1;
    let bps = if h.bits == 16 { 2 } else { 3 };
    let data = &p[4..];
    let n = data.len() / (coded * bps);
    let mut out = vec![Vec::with_capacity(n); h.channels];
    for k in 0..n {
        for (c, o) in out.iter_mut().enumerate() {
            let at = (k * coded + c) * bps;
            let v = if bps == 2 {
                i16::from_be_bytes([data[at], data[at + 1]]) as f32 / 32768.0
            } else {
                (i32::from_be_bytes([data[at], data[at + 1], data[at + 2], 0]) >> 8) as f32 / 8_388_608.0
            };
            o.push(v);
        }
    }
    Some(out)
}

/// Bytes per two sample frames of DVD LPCM. The caller must bound `channels` to the DVD channel range.
pub fn dvd_lpcm_group_bytes(channels: usize, bits: u32) -> usize {
    match bits {
        16 => 4 * channels,
        20 => 5 * channels,
        _ => 6 * channels,
    }
}

/// Decode DVD-Video LPCM (big-endian). 16-bit samples are interleaved; 20- and 24-bit samples
/// come in groups of two frames: the upper 16 bits of all the group's samples, then their low
/// 4 / 8 bits. The caller must bound `channels` to the DVD channel range.
pub fn decode_dvd_lpcm(data: &[u8], channels: usize, bits: u32) -> Vec<Vec<f32>> {
    let ch = channels.max(1);
    let group = dvd_lpcm_group_bytes(ch, bits);
    let groups = data.len() / group;
    let mut out = vec![Vec::with_capacity(groups * 2); ch];
    for g in 0..groups {
        let b = &data[g * group..(g + 1) * group];
        for f in 0..2 {
            for (c, o) in out.iter_mut().enumerate() {
                let k = f * ch + c;
                let hi = i16::from_be_bytes([b[2 * k], b[2 * k + 1]]) as i32;
                let v = match bits {
                    16 => hi as f32 / 32768.0,
                    20 => {
                        let nib = b[4 * ch + k / 2];
                        let lo = if k.is_multiple_of(2) { nib >> 4 } else { nib & 15 } as i32;
                        ((hi << 4) | lo) as f32 / 524_288.0
                    }
                    _ => ((hi << 8) | b[4 * ch + k] as i32) as f32 / 8_388_608.0,
                };
                o.push(v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bluray_header_and_even_channel_padding() {
        let packet = [0, 0, 0x31, 0x40, 0x40, 0, 0xc0, 0];
        assert_eq!(bluray_lpcm_header(&packet), Some(BlurayLpcm { sample_rate: 48_000, channels: 2, bits: 16 }));
        assert_eq!(decode_bluray_lpcm(&packet), Some(vec![vec![0.5], vec![-0.5]]));
        assert_eq!(decode_bluray_lpcm(&packet[..3]), None);
    }
    #[test]
    fn dvd_lpcm_16_20_and_24_bit_groups() {
        assert_eq!(decode_dvd_lpcm(&[0x40, 0, 0xc0, 0], 1, 16), vec![vec![0.5, -0.5]]);
        assert_eq!(decode_dvd_lpcm(&[0x40, 0, 0xc0, 0, 0], 1, 20), vec![vec![0.5, -0.5]]);
        assert_eq!(decode_dvd_lpcm(&[0x40, 0, 0xc0, 0, 0, 0], 1, 24), vec![vec![0.5, -0.5]]);
        assert_eq!(decode_dvd_lpcm(&[1, 2, 3], 1, 16), vec![Vec::<f32>::new()]);
    }
}
