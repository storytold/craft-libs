//! Raw PSD channel DisplayInfo (1077/1007) and Alternate Spot Colors (1067) records.
//!
//! Ported from PhotoCraft channel_map: Jason D's f7f8fdcd89 DisplayInfo framing and
//! Khawar Zamman Wani's 9a390661b5 alternate-spot reader. Colour interpretation, opacity
//! clamping, channel matching and legacy compatibility decisions belong to the caller.
//! Readers borrow their input and ignore a trailing partial record, preserving the
//! original tolerant import behavior. Version words are framing, not validation here.

/// One 13-byte DisplayInfo record, without document-specific colour interpretation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    /// PSD colour-space identifier.
    pub space: u16,
    /// Four raw colour components.
    pub components: [u16; 4],
    /// Raw opacity (normally 0 through 100).
    pub opacity: u16,
    /// Raw channel kind (0 selected, 1 masked, 2 spot).
    pub kind: u8,
}

impl DisplayInfo {
    /// Encode the version-1 resource's 13-byte record, without clamping fields.
    pub fn to_bytes(self) -> [u8; 13] {
        let mut out = [0; 13];
        let values = [self.space, self.components[0], self.components[1], self.components[2], self.components[3], self.opacity];
        for (slot, value) in out[..12].as_chunks_mut::<2>().0.iter_mut().zip(values) {
            *slot = value.to_be_bytes();
        }
        out[12] = self.kind;
        out
    }
}

/// Iterate complete DisplayInfo records without allocating.
///
/// `versioned` selects resource 1077 (skip four version bytes, 13-byte records),
/// otherwise resource 1007 (14-byte records, ignoring each padding byte). A short
/// version header or incomplete final record yields no corresponding record.
pub fn display_info(data: &[u8], versioned: bool) -> impl ExactSizeIterator<Item = DisplayInfo> + '_ {
    let (body, size) = if versioned { (data.get(4..).unwrap_or(&[]), 13) } else { (data, 14) };
    body.chunks_exact(size).map(|rec| {
        let u = |i: usize| u16::from_be_bytes([rec[i], rec[i + 1]]);
        DisplayInfo { space: u(0), components: [u(2), u(4), u(6), u(8)], opacity: u(10), kind: rec[12] }
    })
}

/// One Alternate Spot Colors record, retaining unknown colour spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlternateSpotColor {
    /// Channel identifier from resource 1053.
    pub id: u32,
    /// PSD colour-space identifier.
    pub space: u16,
    /// Four raw colour components.
    pub components: [u16; 4],
}

/// Iterate available complete 14-byte records up to the resource's declared u16 count.
///
/// Short headers, absent declared records and trailing partial bytes are tolerated exactly
/// as in the original importer. No allocation is derived from an untrusted count.
pub fn alternate_spot_colors(data: &[u8]) -> impl ExactSizeIterator<Item = AlternateSpotColor> + '_ {
    let count = data.get(2..4).map_or(0, |c| usize::from(u16::from_be_bytes([c[0], c[1]])));
    data.get(4..).unwrap_or_default().as_chunks::<14>().0.iter().take(count).map(|rec| {
        let u = |i: usize| u16::from_be_bytes([rec[i], rec[i + 1]]);
        AlternateSpotColor { id: u32::from_be_bytes([rec[0], rec[1], rec[2], rec[3]]), space: u(4), components: [u(6), u(8), u(10), u(12)] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_round_trip_and_truncation() {
        let record = DisplayInfo { space: 0x2331, components: [0, 1, 255, u16::MAX], opacity: 500, kind: 255 };
        let mut modern = 99u32.to_be_bytes().to_vec();
        modern.extend(record.to_bytes());
        assert_eq!(display_info(&modern, true).collect::<Vec<_>>(), [record]);
        for n in 0..modern.len() {
            assert_eq!(display_info(&modern[..n], true).len(), 0);
        }
        modern.extend([1, 2, 3]);
        assert_eq!(display_info(&modern, true).len(), 1);
        let mut old = record.to_bytes().to_vec();
        old.push(42);
        old.extend(record.to_bytes());
        old.push(99);
        assert_eq!(display_info(&old, false).collect::<Vec<_>>(), [record, record]);
        assert_eq!(display_info(&old[..27], false).len(), 1);
    }

    #[test]
    fn alternate_count_is_bounded_by_available_records() {
        let mut bytes = vec![0, 9, 255, 255];
        bytes.extend(123u32.to_be_bytes());
        for value in [7u16, 10000, 0xffff, 4, 5] {
            bytes.extend(value.to_be_bytes());
        }
        let want = AlternateSpotColor { id: 123, space: 7, components: [10000, 0xffff, 4, 5] };
        assert_eq!(alternate_spot_colors(&bytes).collect::<Vec<_>>(), [want]);
        for n in 0..bytes.len() {
            assert_eq!(alternate_spot_colors(&bytes[..n]).len(), 0);
        }
        bytes.extend([1, 2, 3]);
        assert_eq!(alternate_spot_colors(&bytes).len(), 1);
        bytes[2..4].copy_from_slice(&0u16.to_be_bytes());
        assert_eq!(alternate_spot_colors(&bytes).len(), 0);
    }
}
