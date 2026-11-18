use super::{Format, Tag};

pub(super) const F: Format = Format::Tiff;

pub(super) const TAG_XMP: u16 = 700;

/// Photoshop image resources (`8BIM` blocks).
pub(super) const TAG_PHOTOSHOP: u16 = 34377;

/// Photoshop `ImageSourceData`: the layers of a layered TIFF.
pub(super) const TAG_IMAGE_SOURCE_DATA: u16 = 37724;

/// `true` when [`encode`] writes little-endian (`II`) files: the `tiff` encoder writes the
/// running target's byte order, and every target PhotoCraft builds for is little-endian.
/// Byte-order-sensitive payloads (Photoshop layer data) must be produced to match.
pub fn writes_little_endian() -> bool {
    cfg!(target_endian = "little")
}

/// `(34377 image resources, 37724 image source data)` payloads, each absent when the tag is.
pub type PhotoshopTags<'a> = (Option<&'a [u8]>, Option<&'a [u8]>);

/// The raw payloads of the Photoshop private tags in the first directory: `(34377 image
/// resources, 37724 image source data)`. Classic and BigTIFF, either byte order; only
/// header bytes and the two payloads are touched, and nothing is allocated. Anything malformed
/// reads as "absent".
pub fn photoshop_tags(b: &[u8]) -> PhotoshopTags<'_> {
    fn walk(b: &[u8]) -> Option<PhotoshopTags<'_>> {
        let le = match b.get(0..2)? {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let u16_at = |o: u64| -> Option<u16> {
            let o = usize::try_from(o).ok()?;
            let s = b.get(o..o.checked_add(2)?)?;
            Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
        };
        let u32_at = |o: u64| -> Option<u32> {
            let o = usize::try_from(o).ok()?;
            let s: [u8; 4] = b.get(o..o.checked_add(4)?)?.try_into().ok()?;
            Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
        };
        let u64_at = |o: u64| -> Option<u64> {
            let o = usize::try_from(o).ok()?;
            let s: [u8; 8] = b.get(o..o.checked_add(8)?)?.try_into().ok()?;
            Some(if le { u64::from_le_bytes(s) } else { u64::from_be_bytes(s) })
        };
        let (big, ifd) = match u16_at(2)? {
            42 => (false, u64::from(u32_at(4)?)),
            43 => (true, u64_at(8)?),
            _ => return None,
        };
        let n = if big { u64_at(ifd)? } else { u64::from(u16_at(ifd)?) }.min(4096);
        let (entry, first, inline) = if big { (20u64, ifd.checked_add(8)?, 8u64) } else { (12, ifd.checked_add(2)?, 4) };
        let mut resources = None;
        let mut layers = None;
        for i in 0..n {
            let e = first.checked_add(i.checked_mul(entry)?)?;
            let tag = u16_at(e)?;
            if tag != TAG_PHOTOSHOP && tag != TAG_IMAGE_SOURCE_DATA {
                continue;
            }
            // BYTE, ASCII, SBYTE or UNDEFINED: one byte per element.
            if !matches!(u16_at(e + 2)?, 1 | 2 | 6 | 7) {
                continue;
            }
            let count = if big { u64_at(e + 4)? } else { u64::from(u32_at(e + 4)?) };
            let value_at = e + if big { 12 } else { 8 };
            let start = if count <= inline {
                value_at
            } else if big {
                u64_at(value_at)?
            } else {
                u64::from(u32_at(value_at)?)
            };
            let start = usize::try_from(start).ok()?;
            let data = b.get(start..start.checked_add(usize::try_from(count).ok()?)?)?;
            if tag == TAG_PHOTOSHOP {
                resources = Some(data);
            } else {
                layers = Some(data);
            }
        }
        Some((resources, layers))
    }
    walk(b).unwrap_or((None, None))
}

/// ASCII tags mapped to `Metadata::text` keys.
pub(super) const TEXT_TAGS: &[(Tag, &str)] = &[
    (Tag::ImageDescription, "Description"),
    (Tag::Make, "Make"),
    (Tag::Model, "Model"),
    (Tag::Software, "Software"),
    (Tag::DateTime, "DateTime"),
    (Tag::Artist, "Artist"),
    (Tag::Copyright, "Copyright"),
];
