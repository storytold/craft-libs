use crate::Kind;
const PLACEABLE_KEY: u32 = 0x9AC6_CDD7;

/// Whether the stream stops before an EOF record header is recognizable.
///
/// This is a diagnostic, not validation: a recognizable EOF header ends
/// the scan even if its payload is incomplete. A parser's resource-limit
/// status is independent of this result.
pub fn ends_early(bytes: &[u8], kind: Kind) -> bool {
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let mut at = match kind {
        Kind::Emf => 0,
        Kind::Wmf => {
            let start = if u32_at(0) == Some(PLACEABLE_KEY) { 22 } else { 0 };
            let Some(words) = u16_at(start + 2) else { return true };
            start + usize::from(words) * 2
        }
    };
    loop {
        let size = match kind {
            Kind::Emf => {
                let (Some(kind), Some(size)) = (u32_at(at), u32_at(at.saturating_add(4))) else { return true };
                if kind == 14 {
                    return false;
                }
                // Records are a multiple of 4 bytes long; reading stops at one that isn't.
                if !size.is_multiple_of(4) {
                    return true;
                }
                size as usize
            }
            Kind::Wmf => {
                let (Some(words), Some(function)) = (u32_at(at), u16_at(at.saturating_add(4))) else { return true };
                if function == 0 {
                    return false;
                }
                (words as usize).saturating_mul(2)
            }
        };
        if size < 6 {
            return true;
        }
        at = at.saturating_add(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emf_eof_header_is_enough_but_other_records_need_aligned_lengths() {
        let mut bytes = [1u32, 8, 14, 20].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
        assert!(!ends_early(&bytes, Kind::Emf));
        assert!(ends_early(&bytes[..12], Kind::Emf));
        bytes[4] = 9;
        assert!(ends_early(&bytes, Kind::Emf));
        bytes[4] = 40;
        assert!(ends_early(&bytes, Kind::Emf));
    }

    #[test]
    fn wmf_eof_follows_standard_and_placeable_headers() {
        for prefix in [0usize, 22] {
            let mut bytes = vec![0u8; prefix + 18];
            if prefix == 22 {
                bytes[..4].copy_from_slice(&PLACEABLE_KEY.to_le_bytes());
            }
            bytes[prefix + 2] = 9;
            bytes.extend_from_slice(&[5, 0, 0, 0, 0, 0]);
            assert!(!ends_early(&bytes, Kind::Wmf));
            assert!(ends_early(&bytes[..bytes.len() - 1], Kind::Wmf));
            let function = bytes.len() - 2;
            bytes[function] = 1;
            assert!(ends_early(&bytes, Kind::Wmf));
        }
    }
}
