use super::*;
use craft_tiff::{IfdBuilder, ImageData, TiffWriter};

#[allow(clippy::unwrap_used)] // Test fixture construction must fail the test if its independent writer fails.
fn plain() -> Vec<u8> {
    let mut raw = IfdBuilder::new();
    for (tag, value) in [
        (t::IMAGE_WIDTH, Value::Long(vec![2])),
        (t::IMAGE_LENGTH, Value::Long(vec![2])),
        (t::BITS_PER_SAMPLE, Value::Short(vec![16])),
        (t::SAMPLES_PER_PIXEL, Value::Short(vec![1])),
        (t::PHOTOMETRIC, Value::Short(vec![photometric::CFA])),
        (t::COMPRESSION, Value::Short(vec![1])),
        (t::AS_SHOT_NEUTRAL, Value::Rational(vec![(1, 2), (1, 1), (2, 3)])),
    ] {
        raw.set(tag, value);
    }
    raw.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![[17u16, 31, 53, 79].into_iter().flat_map(u16::to_le_bytes).collect()] });
    TiffWriter::new(ByteOrder::Little, false).write(&[raw]).unwrap()
}

#[allow(clippy::unwrap_used)] // Only known tags in this module's synthetic TIFF are inspected.
fn entry(bytes: &[u8], tag: u16) -> usize {
    let at = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) as usize;
    (0..count).map(|i| at + 2 + i * 12).find(|&i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap()) == tag).unwrap()
}

#[test]
fn plain_cfa_probe_keeps_malformed_geometry_and_tables_in_the_supported_decoder() {
    let valid = plain();
    assert_eq!(crate::probe(&valid), Some(RawFormat::CfaTiff));
    let decoded = crate::decode(&valid).unwrap();
    assert_eq!(decoded.data, RawData::U16(vec![17, 31, 53, 79]));
    for tag in [t::IMAGE_WIDTH, t::IMAGE_LENGTH] {
        let mut damaged = valid.clone();
        let at = entry(&damaged, tag);
        damaged[at + 8..at + 12].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(crate::probe(&damaged), Some(RawFormat::CfaTiff));
        assert!(matches!(crate::probe_info(&damaged), Err(RawError::Tiff(_))), "zero geometry remains malformed before pixel decode");
        assert!(matches!(crate::decode(&damaged), Err(RawError::Tiff(_))));
    }
    let mut missing = valid.clone();
    let at = entry(&missing, t::STRIP_OFFSETS);
    missing[at..at + 2].copy_from_slice(&0xf123u16.to_le_bytes());
    assert_eq!(crate::probe(&missing), Some(RawFormat::CfaTiff));
    assert!(matches!(crate::probe_info(&missing), Err(RawError::Tiff(_))));
    assert!(matches!(crate::decode(&missing), Err(RawError::Tiff(_))));
}

#[test]
fn plain_cfa_probe_does_not_claim_unknown_tiff_codings_or_missing_color_contracts() {
    for tag in [t::AS_SHOT_NEUTRAL, t::COMPRESSION] {
        let mut unknown = plain();
        let at = entry(&unknown, tag);
        if tag == t::COMPRESSION {
            unknown[at + 8..at + 10].copy_from_slice(&65000u16.to_le_bytes());
        } else {
            unknown[at..at + 2].copy_from_slice(&0xf123u16.to_le_bytes());
        }
        assert_eq!(crate::probe(&unknown), Some(RawFormat::OtherTiff));
        assert!(matches!(crate::decode(&unknown), Err(RawError::Unsupported(_))));
    }
}
