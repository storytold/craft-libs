use craft_psd::ImageResource;
use craft_psd::resources::{ids, unique_exif_xmp};

#[test]
fn metadata_resources_reject_duplicates_and_invalid_xmp() {
    let exif = ImageResource::new(ids::EXIF, b"Exif\0\0II*\0".to_vec());
    let xmp = ImageResource::new(ids::XMP, b"<x/>\0".to_vec());
    let resources = [exif.clone(), xmp.clone()];
    assert_eq!(unique_exif_xmp(&resources), Some((Some(b"Exif\0\0II*\0".as_slice()), Some("<x/>"))));
    assert!(unique_exif_xmp(&[exif.clone(), exif]).is_none());
    assert!(unique_exif_xmp(&[xmp.clone(), xmp]).is_none());
    assert!(unique_exif_xmp(&[ImageResource::new(ids::XMP, vec![255])]).is_none());
    assert_eq!(unique_exif_xmp(&[]), Some((None, None)));
}
