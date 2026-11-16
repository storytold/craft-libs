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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ByteOrder;

    fn fixture(order: ByteOrder, entries: &[(u16, u16, u32, u32)]) -> Vec<u8> {
        let mut bytes = match order {
            ByteOrder::Little => b"II*\0".to_vec(),
            ByteOrder::Big => b"MM\0*".to_vec(),
        };
        order.put_u32(&mut bytes, 8);
        order.put_u16(&mut bytes, u16::try_from(entries.len()).unwrap());
        for &(tag, ty, count, value) in entries {
            order.put_u16(&mut bytes, tag);
            order.put_u16(&mut bytes, ty);
            order.put_u32(&mut bytes, count);
            if ty == 3 {
                order.put_u16(&mut bytes, u16::try_from(value).unwrap());
                order.put_u16(&mut bytes, 0);
            } else {
                order.put_u32(&mut bytes, value);
            }
        }
        bytes
    }

    #[test]
    fn both_byte_orders_and_scalar_types() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            assert_eq!(first_image_compression(&fixture(order, &[])), Some(1));
            for ty in [3, 4] {
                for value in [1, 5, 6, 7, 32773] {
                    let bytes = fixture(order, &[(259, ty, 1, value)]);
                    assert_eq!(first_image_compression(&bytes), u16::try_from(value).ok());
                    // Every prefix missing a required byte is refused; SHORT padding and
                    // the next-IFD pointer are intentionally unnecessary for this probe.
                    let needed = if ty == 3 { 20 } else { 22 };
                    for end in 0..needed {
                        assert_eq!(first_image_compression(&bytes[..end]), None);
                    }
                }
            }
        }
    }

    #[test]
    fn ambiguous_compression_is_refused() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            for pair in [(1, 7), (7, 1), (1, 1)] {
                assert_eq!(first_image_compression(&fixture(order, &[(259, 3, 1, pair.0), (259, 3, 1, pair.1)])), None);
            }
            for entry in [(259, 3, 0, 1), (259, 3, 2, 1), (259, 1, 1, 1), (259, 4, 1, 65536)] {
                assert_eq!(first_image_compression(&fixture(order, &[entry])), None);
            }
            // Unrelated values are not decoded or allocated, including unknown types.
            assert_eq!(first_image_compression(&fixture(order, &[(999, 65535, u32::MAX, u32::MAX)])), Some(1));
            let mut absent = fixture(order, &[(999, 4, 1, 0)]);
            absent.truncate(12); // Only the unrelated tag is needed, matching Word's probe.
            assert_eq!(first_image_compression(&absent), Some(1));
        }
        assert_eq!(first_image_compression(b"II+\0\x08\0\0\0"), None);
        assert_eq!(first_image_compression(b"II*\0\xff\xff\xff\xff"), None);
    }
}
