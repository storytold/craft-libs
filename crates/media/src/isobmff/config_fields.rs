//! Tolerant fields from AVC and HEVC configuration records.

/// NAL length prefix width from AVC configuration, defaulting to four on a short record.
pub fn avcc_length_size(avcc: &[u8]) -> usize {
    avcc.get(4).map_or(4, |b| (b & 3) as usize + 1)
}

/// NAL length prefix width and highest temporal ID from HEVC configuration.
pub fn hvcc_length_size_and_tid(hvcc: &[u8]) -> (usize, Option<u8>) {
    let length_size = hvcc.get(21).map_or(4, |b| (b & 3) as usize + 1);
    let highest_tid = hvcc.get(21).map(|b| (b >> 3) & 7).filter(|&n| n > 0).map(|n| n - 1);
    (length_size, highest_tid)
}

/// objectTypeIndication of the DecoderConfigDescriptor inside an `esds` box in `raw`.
pub fn esds_object_type(raw: &[u8]) -> Option<u8> {
    let at = raw.windows(4).position(|w| w == b"esds")? + 8;
    let mut p = at;
    let size = |p: &mut usize| -> Option<usize> {
        let mut n = 0usize;
        for _ in 0..4 {
            let b = *raw.get(*p)?;
            *p += 1;
            n = (n << 7) | (b & 0x7F) as usize;
            if b & 0x80 == 0 {
                break;
            }
        }
        Some(n)
    };
    if *raw.get(p)? != 0x03 {
        return None;
    }
    p += 1;
    size(&mut p)?;
    let flags = *raw.get(p + 2)?;
    p += 3;
    if flags & 0x80 != 0 {
        p += 2;
    }
    if flags & 0x40 != 0 {
        p += 1 + *raw.get(p)? as usize;
    }
    if flags & 0x20 != 0 {
        p += 2;
    }
    if *raw.get(p)? != 0x04 {
        return None;
    }
    p += 1;
    size(&mut p)?;
    raw.get(p).copied()
}

/// E-AC-3 `ec-3` entry: the program-layout fields are the 24 bits starting at byte 2 of the
/// descriptor, then `bsmod(5) | acmod(3) | lfeon(1)`. Matches FFmpeg's `dec3` demuxer.
pub fn dec3_layout(payload: &[u8]) -> Option<(u32, u32)> {
    if payload.len() < 5 {
        return None;
    }
    let bits = u32::from(payload[2]) << 16 | u32::from(payload[3]) << 8 | u32::from(payload[4]);
    let acmod = ((bits >> 9) & 0x7) as u8;
    let lfeon = bits & 0x100 != 0;
    let channels = dac3_channels(acmod, lfeon)?;
    Some((0, channels))
}

/// AC-3 `ac-3` entry: `dac3` begins with the syncframe bitstream information: sample rate code,
/// bit stream id, bit stream mode, channel mode and LFE flag. The descriptor has no sample rate,
/// so the caller keeps the sample-entry rate.
pub fn dac3_layout(payload: &[u8]) -> Option<(u32, u32)> {
    if payload.len() < 3 {
        return None;
    }
    // The descriptor begins with the bitstream information: fscod(2), bsid(5), bsmod(3),
    // acmod(3) and lfeon(1), continuing in the second payload byte.
    let bits = u32::from_be_bytes([0, 0, payload[1], payload[2]]);
    let acmod = ((bits >> 11) & 0x7) as u8;
    let lfeon = bits & 0x400 != 0;
    let channels = dac3_channels(acmod, lfeon)?;
    Some((0, channels))
}

/// Full-bandwidth channels per AC-3 channel mode (Table 5.8) plus the optional LFE channel.
fn dac3_channels(acmod: u8, lfeon: bool) -> Option<u32> {
    let n = [2u32, 1, 2, 3, 3, 4, 4, 5].get(usize::from(acmod & 7))?.checked_add(u32::from(lfeon))?;
    (n > 0).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_layout_and_elementary_descriptor_are_bounded() {
        assert_eq!(dac3_layout(&[0, 0, 0]), Some((0, 2)));
        assert_eq!(dec3_layout(&[0, 0, 0, 0, 0]), Some((0, 2)));
        assert_eq!(dac3_layout(&[0, 0]), None);
        assert_eq!(dec3_layout(&[0; 4]), None);
        let esds = [b'e', b's', b'd', b's', 0, 0, 0, 0, 0x03, 0x04, 0, 0, 0, 0x04, 0x01, 0x60];
        assert_eq!(esds_object_type(&esds), Some(0x60));
        for end in 0..esds.len() {
            assert_eq!(esds_object_type(&esds[..end]), None);
        }
    }
    #[test]
    fn short_records_and_temporal_layers() {
        assert_eq!(avcc_length_size(&[]), 4);
        assert_eq!(avcc_length_size(&[0, 0, 0, 0, 1]), 2);
        assert_eq!(hvcc_length_size_and_tid(&[]), (4, None));
        let mut h = [0; 22];
        h[21] = (3 << 3) | 2;
        assert_eq!(hvcc_length_size_and_tid(&h), (3, Some(2)));
    }
}
