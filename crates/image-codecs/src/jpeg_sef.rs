//! Validated Samsung SEF trailer repair for JPEGs missing their final EOI marker.
use std::borrow::Cow;

/// Whether the first JPEG image reaches an EOI through its marker and scan structure.
pub fn reaches_eoi(bytes: &[u8]) -> bool {
    crate::codecs::jpeg::reaches_eoi(bytes)
}

/// Borrow ordinary input unchanged. For a validated Samsung SEF trailer following an open JPEG
/// scan, remove that trailer and append the missing EOI. The owned copy is made only on repair.
pub fn repair(bytes: &[u8]) -> Cow<'_, [u8]> {
    let Some(image) = image_end(bytes).and_then(|end| bytes.get(..end)) else { return Cow::Borrowed(bytes) };
    if reaches_eoi(image) {
        return Cow::Borrowed(bytes);
    }
    let mut fixed = Vec::with_capacity(image.len().saturating_add(2));
    fixed.extend_from_slice(image);
    fixed.extend_from_slice(&[0xff, 0xd9]);
    let in_scan = craft_metadata::inspect_jpeg(&fixed).is_some_and(|m| m.first_scan_components > 0) && reaches_eoi(&fixed);
    if in_scan { Cow::Owned(fixed) } else { Cow::Borrowed(bytes) }
}

fn image_end(b: &[u8]) -> Option<usize> {
    let le32 = |at: usize| b.get(at..at.checked_add(4)?)?.try_into().ok().map(|v| u32::from_le_bytes(v) as usize);
    let tail = b.len().checked_sub(8)?;
    if b.get(tail.checked_add(4)?..)? != b"SEFT" {
        return None;
    }
    let dir = tail.checked_sub(le32(tail)?)?;
    if b.get(dir..dir.checked_add(4)?)? != b"SEFH" {
        return None;
    }
    let count = le32(dir.checked_add(8)?)?;
    let entries = dir.checked_add(12)?;
    if count == 0 || count > tail.checked_sub(entries)? / 12 {
        return None;
    }
    let mut start = dir;
    for i in 0..count {
        let entry = entries.checked_add(i.checked_mul(12)?)?;
        let block = dir.checked_sub(le32(entry.checked_add(4)?)?)?;
        let len = le32(entry.checked_add(8)?)?;
        let kind = |at: usize| b.get(at.checked_add(2)?..at.checked_add(4)?);
        if len < 4 || block.checked_add(len)? > dir || kind(block)? != kind(entry)? {
            return None;
        }
        start = start.min(block);
    }
    Some(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repairs_only_valid_sef_after_an_open_scan() {
        let image = [0xff, 0xd8, 0xff, 0xc0, 0, 8, 8, 0, 1, 0, 1, 1, 0xff, 0xda, 0, 3, 1, 0x12, 0xff, 0, 0x34];
        let mut b = image.to_vec();
        let block = [0, 0, 0xe0, 0x08, 1, 2, 3, 4];
        b.extend_from_slice(&block);
        b.extend_from_slice(b"SEFH");
        b.extend_from_slice(&106u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[0, 0, 0xe0, 0x08]);
        b.extend_from_slice(&(block.len() as u32).to_le_bytes());
        b.extend_from_slice(&(block.len() as u32).to_le_bytes());
        b.extend_from_slice(&24u32.to_le_bytes());
        b.extend_from_slice(b"SEFT");
        let mut expected = image.to_vec();
        expected.extend_from_slice(&[0xff, 0xd9]);
        assert_eq!(repair(&b).as_ref(), expected);
        assert!(matches!(repair(&image), Cow::Borrowed(_)));
        b[image.len() + 2] = 0;
        assert!(matches!(repair(&b), Cow::Borrowed(_)));
    }
    #[test]
    fn eoi_requires_valid_lengths_and_skips_stuffed_bytes() {
        assert!(reaches_eoi(&[0xff, 0xd8, 0xff, 0, 0xff, 0xd9]));
        assert!(!reaches_eoi(&[0xff, 0xd8, 0xff, 0xe1, 0, 1, 0xff, 0xd9]));
        assert!(!reaches_eoi(&[0xff, 0xd8, 0xff, 0xe1, 0, 4, 0xff, 0xd9]));
    }
}
