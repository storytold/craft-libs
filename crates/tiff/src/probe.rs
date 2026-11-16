//! Allocation-free classic TIFF compression inspection.

/// Return the first image's compression, or 1 when its tag is absent.
///
/// Only classic TIFF is accepted. A duplicate Compression tag, non-scalar value,
/// unsupported field type, out-of-range LONG, or unreadable required field returns
/// `None`. Duplicate tags are refused rather than choosing one: decoders may use
/// a different copy, bypassing a caller's compression/resource policy.
///
/// This inspects only compression, not image validity. Unrelated tag payloads,
/// next-IFD pointers and image data are not read. The caller chooses allowed
/// compression values and must still validate/decode the image with its limits.
/// No allocation occurs; at most 65,535 first-directory entries are inspected.
pub fn first_image_compression(bytes: &[u8]) -> Option<u16> {
    let le = match bytes.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = [*bytes.get(at)?, *bytes.get(at.checked_add(1)?)?];
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let mut found = None;
    for i in 0..usize::from(u16_at(ifd)?) {
        let entry = ifd.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        if u16_at(entry)? != 259 {
            continue;
        }
        if found.is_some() || u32_at(entry.checked_add(4)?)? != 1 {
            return None;
        }
        found = Some(match u16_at(entry.checked_add(2)?)? {
            3 => u16_at(entry.checked_add(8)?)?,
            4 => u16::try_from(u32_at(entry.checked_add(8)?)?).ok()?,
            _ => return None,
        });
    }
    Some(found.unwrap_or(1))
}
