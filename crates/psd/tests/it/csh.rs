use craft_psd::PsdError;
use craft_psd::csh;
use craft_psd::path::{PathData, PsdKnot, PsdSubpath};

fn fixture(names: &[&str]) -> Vec<u8> {
    let mut bytes = b"cush".to_vec();
    bytes.extend_from_slice(&2u32.to_be_bytes());
    bytes.extend_from_slice(&(names.len() as u32).to_be_bytes());
    for name in names {
        let units: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        bytes.extend_from_slice(&(units.len() as u32).to_be_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        bytes.extend_from_slice(&1u32.to_be_bytes());
        let mut body = vec![2, b'i', b'd'];
        for bound in [-10i32, -20, 90, 180] {
            body.extend_from_slice(&bound.to_be_bytes());
        }
        let knot = PsdKnot { linked: true, pre: (0.125, 0.25), anchor: (0.5, 0.5), post: (0.75, 0.875) };
        let path = PathData::from_subpaths(&[PsdSubpath { closed: false, operation: 0, knots: vec![knot] }], false);
        body.extend_from_slice(&path.to_bytes());
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&body);
    }
    bytes
}

#[test]
fn reads_multiple_names_padding_signed_bounds_and_cubic_controls() {
    let bytes = fixture(&["Even", "Odd", "形😀"]);
    let shapes = csh::parse(&bytes).unwrap();
    assert_eq!(shapes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Even", "Odd", "形😀"]);
    for shape in shapes {
        assert_eq!(shape.bounds, [-10, -20, 90, 180]);
        assert_eq!(shape.id, "id");
        let subpaths = shape.path.subpaths();
        assert_eq!(subpaths.len(), 1);
        assert!(!subpaths[0].closed);
        let knot = subpaths[0].knots[0];
        assert!(knot.linked);
        assert_eq!(knot.pre, (0.125, 0.25));
        assert_eq!(knot.anchor, (0.5, 0.5));
        assert_eq!(knot.post, (0.75, 0.875));
    }
}

#[test]
fn rejects_every_truncated_prefix_without_panicking() {
    let bytes = fixture(&["One", "Two"]);
    for end in 0..bytes.len() {
        assert!(csh::parse(&bytes[..end]).is_err(), "accepted truncated prefix {end}");
    }
}

#[test]
fn rejects_unsupported_headers_excessive_counts_and_invalid_unicode() {
    let bytes = fixture(&["A"]);
    for (offset, value) in [(4, 3u32), (8, u32::MAX), (12, u32::MAX), (24, u32::MAX)] {
        let mut broken = bytes.clone();
        broken[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert!(csh::parse(&broken).is_err(), "offset {offset}");
    }
    // The count and name limits answer before the reader runs out of bytes, so a removed limit
    // cannot hide behind a truncation error. One past each limit is refused by the limit itself.
    for (offset, value, limit) in [
        (8, u32::MAX, "CSH shape count"),
        (8, csh::MAX_SHAPES as u32 + 1, "CSH shape count"),
        (12, u32::MAX, "CSH name length"),
        (12, csh::MAX_NAME_UNITS as u32 + 1, "CSH name length"),
    ] {
        let mut broken = bytes.clone();
        broken[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert_eq!(csh::parse(&broken), Err(PsdError::LimitExceeded(limit)), "offset {offset} value {value}");
    }
    let mut broken = bytes.clone();
    broken[16..18].copy_from_slice(&0xd800u16.to_be_bytes());
    assert!(csh::parse(&broken).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(csh::parse(&trailing).is_err());
}

#[test]
fn rejects_missing_knots_and_mismatched_open_closed_records() {
    let bytes = fixture(&["A"]);
    // Header/name/version/length = 28; id = 3; bounds = 16; two fill records = 52.
    let length_record = 28 + 3 + 16 + 52;
    let mut missing = bytes.clone();
    missing[length_record + 2..length_record + 4].copy_from_slice(&2u16.to_be_bytes());
    assert!(csh::parse(&missing).is_err());
    let mut mismatched = bytes;
    mismatched[length_record..length_record + 2].copy_from_slice(&0u16.to_be_bytes());
    assert!(csh::parse(&mismatched).is_err());
}
