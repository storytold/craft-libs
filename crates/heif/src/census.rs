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
    Ok(())
}

/// The metadata-box payload sizes seen so far.
#[derive(Default)]
struct Census {
    ipma: usize,
    iloc: usize,
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

    #[test]
    fn a_undersized_ipma_is_untouched_by_the_caps() {
        // The same shape with a normal ipma (two entries): the caps pass, and whatever error
        // follows must not come from them.
        let bytes = file_with(&[0, 0, 4, 1, 1, 2, 0, 0]);
        let r = crate::probe(&bytes);
        assert!(!matches!(r, Err(Error::Limit(ref m)) if m.contains("cap")), "{r:?}");
    }
}
