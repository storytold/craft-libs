//! Caps on the container's metadata boxes, counted before heic-rs touches the file.
//!
//! heic-rs 0.1.4 (pinned `=0.1.4`) has no aggregate work budget inside its container parsing:
//! every `ipma` association entry rescans the vector of already-seen item ids (quadratic in
//! the number of entries), and an `iloc` extent using construction method 2 reconstructs its
//! source item afresh for every extent that references it, with no shared cache. Legitimate
//! phone-photo metadata never comes near these caps — real corpora keep both well under a few
//! kilobytes — so anything above them is hostile or corrupt and is refused before the
//! countable CPU work happens.
//!
//! [`depth_boxes`] stops silently at structural blips: heic-rs's own parser reports those
//! better than this walk could, and an `ipma` box past the cap is caught wherever it parses.

use heic_rs::boxes::BoxIter;

use crate::Error;

/// The largest `ipma` box (item ↔ property associations) payload accepted, 64 KiB. An entry
/// costs at least 3 bytes, so this bounds heic-rs's per-entry rescan of earlier item ids to
/// ~21 000 entries ≈ 220 million comparisons: long enough to be annoying, never minutes.
const MAX_IPMA_BYTES: usize = 64 << 10;

/// The largest `iloc` box (item locations) payload accepted, 64 KiB. An extent costs at least
/// 12 bytes, so this bounds both the parse itself and how many extents a
/// construction-method-2 reconstruction chain can visit.
const MAX_ILOC_BYTES: usize = 64 << 10;

/// The most extents accepted from an `iloc` whose offset, length and index fields are all
/// zero-width (same as the item-visit budget). Such an extent takes no bytes, so the payload cap
/// does not bound it, yet heic-rs 0.1.4 skips its "more extents than it holds" check and
/// allocates 24 bytes per extent (up to 65535 per item).
const MAX_ZERO_WIDTH_EXTENTS: u64 = 4096;

/// Falls through unless the container declares more `ipma` or `iloc` payload than a
/// legitimate phone photo ever does.
pub fn cap(bytes: &[u8]) -> Result<(), Error> {
    let mut c = Census::default();
    depth_boxes(&mut c, BoxIter::new(bytes), 0);
    if c.ipma > MAX_IPMA_BYTES {
        return Err(Error::Limit(format!("the image carries {} bytes of property associations, over the {MAX_IPMA_BYTES}-byte cap", c.ipma)));
    }
    if c.iloc > MAX_ILOC_BYTES {
        return Err(Error::Limit(format!("the image carries {} bytes of item locations, over the {MAX_ILOC_BYTES}-byte cap", c.iloc)));
    }
    if c.zero_width_extents > MAX_ZERO_WIDTH_EXTENTS {
        return Err(Error::Limit(format!(
            "the image declares {} extents that occupy no bytes, over the {MAX_ZERO_WIDTH_EXTENTS}-extent cap",
            c.zero_width_extents
        )));
    }
    Ok(())
}

/// The metadata-box payload sizes seen so far.
#[derive(Default)]
struct Census {
    ipma: usize,
    iloc: usize,
    zero_width_extents: u64,
}

/// The number of extents an `iloc` payload declares when its extents are zero-width, else 0.
/// Stops silently at a truncated or unknown layout: heic-rs reports those itself.
fn zero_width_extents(p: &[u8]) -> u64 {
    let (Some(&version), Some(&sizes), Some(&sizes2)) = (p.first(), p.get(4), p.get(5)) else { return 0 };
    let base_size = usize::from(sizes2 >> 4);
    let index_size = if version >= 1 { sizes2 & 0x0f } else { 0 };
    if version > 2 || (sizes >> 4) + (sizes & 0x0f) + index_size != 0 {
        return 0;
    }
    let id_size = if version < 2 { 2 } else { 4 };
    let Some(count) = p.get(6..6 + id_size) else { return 0 };
    let count = count.iter().fold(0u32, |n, &b| n << 8 | u32::from(b));
    let (mut at, mut total) = (6 + id_size, 0u64);
    for _ in 0..count {
        // id, [construction method], data reference index, base offset, then the extent count
        at += id_size + if version >= 1 { 2 } else { 0 } + 2 + base_size;
        let Some(n) = p.get(at..at + 2) else { break };
        total += u64::from(u16::from_be_bytes([n[0], n[1]]));
        at += 2;
    }
    total
}

/// Walks boxes, adding the payload sizes of those named in [`Census`].
///
/// Only `meta` and `iprp` (the two nesting boxes the metadata is built from) are descended
/// into, no deeper than that depth, so a nested-box trick can't hide a box from the count.
fn depth_boxes(c: &mut Census, boxes: BoxIter<'_>, depth: u32) {
    if depth > 3 {
        return;
    }
    for b in boxes {
        // A structural error stops the walk; `heic_rs::probe` reports it properly afterwards.
        let Ok(b) = b else { return };
        if b.is(b"ipma") {
            c.ipma = c.ipma.saturating_add(b.payload.len());
        } else if b.is(b"iloc") {
            c.iloc = c.iloc.saturating_add(b.payload.len());
            c.zero_width_extents = c.zero_width_extents.saturating_add(zero_width_extents(b.payload));
        } else if b.is(b"meta") {
            // `meta` is a FullBox: version and flags sit between its header and its children.
            if b.payload.len() >= 4 {
                depth_boxes(c, BoxIter::new(&b.payload[4..]), depth + 1);
            }
        } else if b.is(b"iprp") {
            depth_boxes(c, b.children(), depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assembles ftyp + meta(version/flags + iprp(ipco, ipma)) by hand; only the structure
    /// the census walks matters, no item is ever defined.
    fn file_with(ipma_payload: &[u8]) -> Vec<u8> {
        let mut out = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic".to_vec();
        let mut iprp = Vec::new();
        iprp.extend(box_of(b"ipco", b""));
        iprp.extend(box_of(b"ipma", ipma_payload));
        let mut meta = vec![0u8, 0, 0, 0]; // version 0, flags 0
        meta.extend(box_of(b"iprp", &iprp));
        out.extend(box_of(b"meta", &meta));
        out
    }

    fn box_of(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + body.len());
        let size = u32::try_from(8 + body.len()).expect("test box fits in u32");
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn an_oversized_ipma_is_refused() {
        // 40 000 entries of 6 bytes: 240 KiB of ipma, far over the cap, and quadratic if
        // parsed (each entry rescans the earlier item ids in heic-rs 0.1.4).
        let mut ipma = Vec::new();
        for _ in 0..40_000u32 {
            ipma.extend_from_slice(&[0u8, 0, 0, 0, 0, 0]);
        }
        let bytes = file_with(&ipma);
        let r = crate::probe(&bytes).map(|_| ());
        assert!(matches!(r, Err(Error::Limit(ref m)) if m.contains("associations")), "{r:?}");
    }

    /// An `iloc` v0 whose offset, length and base-offset sizes are 0, listing `items` items of
    /// `extents` extents each. Each item is 8 bytes, so 4096+ items stay far under the byte cap.
    fn zero_width_iloc(items: u16, extents: u16) -> Vec<u8> {
        let mut iloc = vec![0u8, 0, 0, 0, 0x00, 0x00];
        iloc.extend_from_slice(&items.to_be_bytes());
        for id in 0..items {
            iloc.extend_from_slice(&id.to_be_bytes());
            iloc.extend_from_slice(&[0, 0]); // data reference index
            iloc.extend_from_slice(&extents.to_be_bytes());
        }
        iloc
    }

    fn file_with_iloc(iloc: &[u8]) -> Vec<u8> {
        let mut out = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic".to_vec();
        let mut meta = vec![0u8, 0, 0, 0];
        meta.extend(box_of(b"iloc", iloc));
        out.extend(box_of(b"meta", &meta));
        out
    }

    #[test]
    fn zero_width_extents_are_counted_not_allocated() {
        // 8000 items x 65535 extents would be ~12 GB in heic-rs 0.1.4; the census only counts.
        let iloc = zero_width_iloc(8000, u16::MAX);
        assert!(iloc.len() < MAX_ILOC_BYTES);
        let r = cap(&file_with_iloc(&iloc));
        assert!(matches!(r, Err(Error::Limit(ref m)) if m.contains("occupy no bytes")), "{r:?}");
        // at the cap it passes; one more is refused
        assert_eq!(zero_width_extents(&zero_width_iloc(4096, 1)), 4096);
        assert!(cap(&file_with_iloc(&zero_width_iloc(4096, 1))).is_ok());
        assert!(cap(&file_with_iloc(&zero_width_iloc(4097, 1))).is_err());
    }

    #[test]
    fn the_thumbnail_path_applies_the_caps_too() {
        let bytes = file_with_iloc(&zero_width_iloc(8000, u16::MAX));
        let r = crate::decode_thumbnail(&bytes, 1, &crate::Options::default()).map(|_| ());
        assert!(matches!(r, Err(Error::Limit(ref m)) if m.contains("occupy no bytes")), "{r:?}");
    }

    #[test]
    fn extents_with_width_are_left_to_the_byte_cap() {
        // offset_size 4, length_size 4: the payload cap already bounds them.
        let mut iloc = zero_width_iloc(1, u16::MAX);
        iloc[4] = 0x44;
        assert_eq!(zero_width_extents(&iloc), 0);
    }

    #[test]
    fn a_undersized_ipma_is_untouched_by_the_caps() {
        // The same shape with a normal ipma (two entries): the caps pass, and whatever error
        // follows must not come from them.
        let bytes = file_with(&[0, 0, 4, 1, 1, 2, 0, 0]);
        let r = crate::probe(&bytes);
        assert!(!matches!(r, Err(Error::Limit(ref m)) if m.contains("cap")), "{r:?}");
    }
}
