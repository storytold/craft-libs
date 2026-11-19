//! Raw alpha-channel name record framing for image resources 1006 and 1045.
//!
//! Callers choose text encoding, replacement characters and Unicode termination. Resource
//! container length checks and padding remain in the image-resource writer.

/// Append one counted big-endian UTF-16 record for resource 1045.
///
/// Units are copied unchanged, including unpaired surrogates and embedded NULs. No terminator
/// or padding is added. The count uses the existing shared Unicode writer's wire conversion;
/// this does not relax the enclosing resource's length validation.
pub fn append_unicode_record(out: &mut Vec<u8>, units: &[u16]) {
    crate::io::write_unicode_units(out, units);
}

/// Append one raw Pascal name for resource 1006, truncating to 255 bytes.
///
/// This adds only the length byte and name bytes, with no per-record padding. Text encoding
/// and replacement policy belong to the caller; the enclosing resource pads its payload.
pub fn append_pascal_record(out: &mut Vec<u8>, bytes: &[u8]) {
    crate::io::write_pascal(out, bytes, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_appends_literal_raw_units_without_termination_or_padding() {
        let mut out = vec![0xaa];
        append_unicode_record(&mut out, &[]);
        append_unicode_record(&mut out, &[0]);
        append_unicode_record(&mut out, &[0x0041, 0, 0xd800, 0xdc00, 0xdfff]);
        assert_eq!(out, [0xaa, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 5, 0, 0x41, 0, 0, 0xd8, 0, 0xdc, 0, 0xdf, 0xff]);
    }

    #[test]
    fn pascal_appends_literal_records_without_padding() {
        let mut out = vec![0xaa];
        append_pascal_record(&mut out, &[]);
        append_pascal_record(&mut out, &[b'A', 0, 0xff]);
        append_pascal_record(&mut out, b"BC");
        assert_eq!(out, [0xaa, 0, 3, b'A', 0, 0xff, 2, b'B', b'C']);
        for len in [254, 255, 256] {
            let bytes = vec![0xfe; len];
            let mut out = Vec::new();
            append_pascal_record(&mut out, &bytes);
            assert_eq!(usize::from(out[0]), len.min(255));
            assert_eq!(&out[1..], &bytes[..len.min(255)]);
        }
    }

    #[test]
    fn outer_resource_pads_odd_payload_not_individual_names() {
        let mut data = Vec::new();
        append_pascal_record(&mut data, b"AB");
        let resource = crate::resources::ImageResource::new(1006, data);
        let mut out = Vec::new();
        resource.write(&mut out).unwrap();
        assert_eq!(out, [b'8', b'B', b'I', b'M', 3, 0xee, 0, 0, 0, 0, 0, 3, 2, b'A', b'B', 0]);
    }
}
