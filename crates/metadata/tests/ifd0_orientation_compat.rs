#[test]
fn missing_and_invalid_values_remain_distinct() {
    let mut data = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".to_vec();
    assert_eq!(craft_metadata::ifd0_orientation_compat(&data).map(|o| o.to_exif()), Some(6));
    data[18] = 0;
    assert_eq!(craft_metadata::ifd0_orientation_compat(&data).map(|o| o.to_exif()), Some(1));
    assert_eq!(craft_metadata::ifd0_orientation_compat(b"invalid"), None);
}
