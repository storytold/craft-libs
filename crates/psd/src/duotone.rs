//! Raw records in Photoshop's version 1 Duotone Color Mode Data.
//!
//! Layout extracted from PhotoCraft `duotone_map.rs`, originally by Gert Pellin
//! (772cf39c09930229e11b920196f486239bbbfab8), with colour-book integration by David Mann.
//! Colour conversion, curve sampling and fallback/display policy are caller-owned.
//! Parsing accepts the 412-byte record prefix as PhotoCraft did; dot gain and overprints
//! are optional trailing data, not prerequisites for interpreting the inks.

/// Full block size emitted by Photoshop and PhotoCraft.
pub const BLOCK_LEN: usize = 524;
/// Alternate Duotone Colors image resource.
pub const ALTERNATE_DUOTONE_COLORS: u16 = 1066;
/// Raw colour space and four component words, without colour interpretation.
pub type ColorRecord = [u16; 5];
/// Sentinel for an absent ink or computed overprint colour.
pub const NO_COLOR: ColorRecord = [0xffff, 0, 0, 0, 0];

/// Valid active ink counts for a serialized block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum InkCount {
    /// One active ink.
    One = 1,
    /// Two active inks.
    Two = 2,
    /// Three active inks.
    Three = 3,
    /// Four active inks.
    Four = 4,
}

/// One fixed-size ink slot, including uninterpreted reserved curve word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InkRecord {
    /// Raw ink colour space and components.
    pub color: ColorRecord,
    /// Pascal name bytes, including its length byte; text encoding belongs to the caller.
    pub name: [u8; 64],
    /// Thirteen signed curve samples and the reserved final word.
    pub curve: [i16; 14],
}

fn word(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

/// Decode a raw colour record at a byte offset, without guessing unknown colour spaces.
pub fn color_at(data: &[u8], at: usize) -> Option<ColorRecord> {
    let bytes = data.get(at..at.checked_add(10)?)?;
    Some(std::array::from_fn(|i| u16::from_be_bytes([bytes[i * 2], bytes[i * 2 + 1]])))
}

/// Borrowed, validated view over at most four active ink records. No input-sized allocation.
#[derive(Clone, Copy, Debug)]
pub struct DuotoneView<'a> {
    data: &'a [u8],
    count: usize,
}

impl<'a> DuotoneView<'a> {
    /// Accept version 1, one to four inks and the full 412-byte record prefix.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 412 || word(data, 0)? != 1 {
            return None;
        }
        let count = usize::from(word(data, 2)?);
        (1..=4).contains(&count).then_some(Self { data, count })
    }

    /// Number of active ink records (one through four).
    pub fn ink_count(self) -> usize {
        self.count
    }

    /// Copy one small fixed-size record. Inactive/out-of-range slots are not returned.
    pub fn ink(self, index: usize) -> Option<InkRecord> {
        if index >= self.count {
            return None;
        }
        let name_at = 44 + 64 * index;
        let name = self.data.get(name_at..name_at + 64)?.try_into().ok()?;
        let curve_at = 300 + 28 * index;
        let curve_bytes = self.data.get(curve_at..curve_at + 28)?;
        Some(InkRecord {
            color: color_at(self.data, 4 + 10 * index)?,
            name,
            curve: std::array::from_fn(|i| i16::from_be_bytes([curve_bytes[i * 2], curve_bytes[i * 2 + 1]])),
        })
    }
}

/// Read one raw alternate colour when the resource version/count permits it.
pub fn alternate_color(data: &[u8], index: usize) -> Option<ColorRecord> {
    if word(data, 0)? != 1 || index >= usize::from(word(data, 2)?) {
        return None;
    }
    color_at(data, 4usize.checked_add(index.checked_mul(10)?)?)
}

/// Serialize four raw slots and explicit trailing policy into a fixed-size block.
/// The typed count cannot be invalid. Reserved bytes and inactive slots are preserved as supplied.
pub fn write_block(count: InkCount, inks: &[InkRecord; 4], dot_gain: u16, overprints: &[ColorRecord; 11]) -> [u8; BLOCK_LEN] {
    let mut out = [0u8; BLOCK_LEN];
    out[..2].copy_from_slice(&1u16.to_be_bytes());
    out[2..4].copy_from_slice(&(count as u16).to_be_bytes());
    for (i, ink) in inks.iter().enumerate() {
        for (j, value) in ink.color.iter().enumerate() {
            let at = 4 + 10 * i + 2 * j;
            out[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
        out[44 + 64 * i..44 + 64 * (i + 1)].copy_from_slice(&ink.name);
        for (j, value) in ink.curve.iter().enumerate() {
            let at = 300 + 28 * i + 2 * j;
            out[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
    }
    out[412..414].copy_from_slice(&dot_gain.to_be_bytes());
    for (i, color) in overprints.iter().enumerate() {
        for (j, value) in color.iter().enumerate() {
            let at = 414 + 10 * i + 2 * j;
            out[at..at + 2].copy_from_slice(&value.to_be_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_layout_roundtrip_preserves_raw_fields() {
        let mut ink = InkRecord { color: [7, 1, 2, 3, 65535], name: [0; 64], curve: [-1; 14] };
        ink.name[..4].copy_from_slice(&[3, b'I', b'n', b'k']);
        ink.curve[0] = 0;
        ink.curve[12] = 1000;
        let slots = std::array::from_fn(|_| ink.clone());
        let bytes = write_block(InkCount::Four, &slots, 20, &[NO_COLOR; 11]);
        assert_eq!(&bytes[..4], &[0, 1, 0, 4]);
        assert_eq!(&bytes[44..48], &[3, b'I', b'n', b'k']);
        assert_eq!(&bytes[412..416], &[0, 20, 255, 255]);
        let view = DuotoneView::parse(&bytes);
        assert_eq!(view.map(DuotoneView::ink_count), Some(4));
        for i in 0..4 {
            assert_eq!(view.and_then(|v| v.ink(i)), Some(ink.clone()));
        }
        assert!(view.and_then(|v| v.ink(usize::MAX)).is_none());
        for n in 0..412 {
            assert!(DuotoneView::parse(&bytes[..n]).is_none());
        }
        assert!(DuotoneView::parse(&bytes[..412]).is_some());
    }

    #[test]
    fn rejects_hostile_offsets_versions_counts_and_alternate_truncation() {
        let mut bytes = [0u8; BLOCK_LEN];
        bytes[..4].copy_from_slice(&[0, 1, 255, 255]);
        assert!(DuotoneView::parse(&bytes).is_none());
        bytes[..4].copy_from_slice(&[0, 2, 0, 1]);
        assert!(DuotoneView::parse(&bytes).is_none());
        assert!(color_at(&bytes, usize::MAX).is_none());
        let alt = [0, 1, 0, 1, 0, 7, 0, 1, 0, 2, 0, 3, 0, 4];
        assert_eq!(alternate_color(&alt, 0), Some([7, 1, 2, 3, 4]));
        assert!(alternate_color(&alt[..13], 0).is_none());
        assert!(alternate_color(&alt, 1).is_none());
        assert!(alternate_color(&alt, usize::MAX).is_none());
    }
}
