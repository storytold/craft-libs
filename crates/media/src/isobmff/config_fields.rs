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

#[cfg(test)]
mod tests {
    use super::*;
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
