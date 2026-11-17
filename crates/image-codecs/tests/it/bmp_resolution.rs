//! A BMP keeps its resolution in the header's `biXPelsPerMeter` and `biYPelsPerMeter`, which
//! Photoshop writes and reads (storytold/photocraft#13630). BMP exports left both 0, so a 300 ppi
//! document saved as BMP opened at 72 ppi elsewhere, and an opened BMP's resolution was ignored.

use crate::common::test_image;
use craft_image_codecs::*;

/// The DIB header's size and its pixels per metre, horizontal then vertical (file offsets 14, 38
/// and 42, little-endian).
fn header(bytes: &[u8]) -> (u32, i32, i32) {
    let at = |i: usize| [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]];
    (u32::from_le_bytes(at(14)), i32::from_le_bytes(at(38)), i32::from_le_bytes(at(42)))
}

/// A test image of `layout` at resolution `dpi`.
fn image(layout: ChannelLayout, dpi: Option<(f32, f32)>) -> Image {
    let mut img = test_image(layout, SampleType::U8);
    img.meta.dpi = dpi;
    img
}

#[test]
fn bmp_export_writes_the_resolution_in_pixels_per_metre_and_reads_it_back() {
    // RGB is written with a BITMAPINFOHEADER, RGBA with a BITMAPV4HEADER: same field offsets.
    for (layout, dib) in [(ChannelLayout::Rgb, 40), (ChannelLayout::Rgba, 108)] {
        // The fields are part of the header, not metadata: embed_metadata off keeps them.
        for embed_metadata in [true, false] {
            let opts = EncodeOptions { embed_metadata, ..Default::default() };
            let img = image(layout, Some((300.0, 150.0)));
            let bytes = encode(&img, Format::Bmp, &opts).unwrap();
            // 300 / 0.0254 = 11811.02 and 150 / 0.0254 = 5905.51, rounded.
            assert_eq!(header(&bytes), (dib, 11811, 5906), "{layout:?} metadata {embed_metadata}");
            assert_eq!(decode(&bytes).unwrap().meta.dpi, Some((300.0, 150.0)), "{layout:?} metadata {embed_metadata}");
            assert!(!fidelity_warnings_with(&img, Format::Bmp, &opts).contains(&FidelityWarning::DpiDropped), "{layout:?}");
        }
    }
}

#[test]
fn bmp_without_a_resolution_leaves_the_fields_unset() {
    let bytes = encode(&image(ChannelLayout::Rgb, None), Format::Bmp, &EncodeOptions::default()).unwrap();
    assert_eq!(header(&bytes), (40, 0, 0));
    assert_eq!(decode(&bytes).unwrap().meta.dpi, None, "0 means no resolution: the caller's default applies");
}

#[test]
fn bmp_opens_at_the_resolution_its_header_records() {
    let bytes = encode(&image(ChannelLayout::Rgb, None), Format::Bmp, &EncodeOptions::default()).unwrap();
    let with = |x: i32, y: i32| {
        let mut b = bytes.clone();
        b[38..42].copy_from_slice(&x.to_le_bytes());
        b[42..46].copy_from_slice(&y.to_le_bytes());
        decode(&b).unwrap().meta.dpi
    };
    // What other writers store for 96 and 72 ppi reads back whole.
    assert_eq!(with(3780, 3780), Some((96.0, 96.0)));
    assert_eq!(with(2835, 2835), Some((72.0, 72.0)));
    // 40000 px/m is exactly 1016 ppi; a value no whole ppi rounds to stays as it is (71.98).
    let (x, y) = with(40000, 2834).unwrap();
    assert_eq!(x, 1016.0);
    assert!((y - 71.9836).abs() < 1e-3, "{y}");
    // An axis that is 0 or negative leaves the resolution unknown.
    assert_eq!(with(11811, 0), None);
    assert_eq!(with(-11811, 11811), None);
}

#[test]
fn indexed_bmp_writes_the_resolution_it_is_given() {
    let (idx, pal) = ([0u8, 1, 1, 0], [[0, 0, 0], [255, 0, 0]]);
    let bytes = encode_bmp_indexed_with_dpi(2, 2, &idx, &pal, Some((300.0, 300.0))).unwrap();
    assert_eq!(header(&bytes), (40, 11811, 11811));
    assert_eq!(decode(&bytes).unwrap().meta.dpi, Some((300.0, 300.0)));
    let unset = encode_bmp_indexed_with_dpi(2, 2, &idx, &pal, None).unwrap();
    assert_eq!(header(&unset), (40, 0, 0));
    // The existing function keeps writing no resolution, so its callers' files don't change.
    assert_eq!(encode_bmp_indexed(2, 2, &idx, &pal).unwrap(), unset);
}
