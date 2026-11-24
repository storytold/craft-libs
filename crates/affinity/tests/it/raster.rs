//! Pixel retention on current Affinity 3 documents published under MIT by Patchy.
//! Assertions use this reader's archive/stream and the original embedded image bytes;
//! no third-party Affinity parser, source code or application is used as an oracle.

use craft_affinity::{
    Archive, Limits,
    model::{Affine, Document, Image, Kind, Node, Pixels},
    stream,
};

use crate::corpus;

fn images(nodes: &[Node], out: &mut Vec<Image>) {
    for node in nodes {
        if let Kind::Image(image) = &node.kind {
            out.push(image.clone());
        }
        images(&node.children, out);
    }
}

fn read_fixture(name: &str) -> Option<(Vec<u8>, Document, Vec<Image>)> {
    let dir = corpus::pinned()?;
    let bytes = std::fs::read(dir.join(name)).unwrap();
    let doc = craft_affinity::read(&bytes, Limits::default()).unwrap();
    assert!(doc.saved_by.as_deref().unwrap().starts_with("Affinity 3."));
    let mut out = Vec::new();
    for spread in &doc.spreads {
        images(&spread.nodes, &mut out);
    }
    Some((bytes, doc, out))
}

fn source(bytes: &[u8]) -> Vec<u8> {
    let mut archive = Archive::open(bytes, Limits::default()).unwrap();
    let doc = stream::parse(&archive.read("doc.dat").unwrap()).unwrap();
    let bitmap =
        doc.objects.iter().enumerate().find(|(_, o)| o.class == stream::Tag::of(b"DyBm") && o.get(stream::Tag::of(b"Bckg")).is_some()).unwrap().0;
    let name = doc.entry(bitmap, b"Bckg").unwrap();
    let block = stream::parse(&archive.read(name).unwrap()).unwrap();
    block.bytes(block.root, b"Data").unwrap().to_vec()
}

#[test]
fn source_backed_tiles_retain_the_embedded_jpeg_pixels() {
    let Some((bytes, doc, images)) = read_fixture("patchy-embedded-jpeg.af") else { return };
    assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
    assert_eq!(images.len(), 1);
    let image = &images[0];
    assert_eq!((image.width, image.height), (400, 300));
    assert_eq!(image.transform, Affine::IDENTITY);
    let Pixels::Rgba8(pixels) = &image.pixels else { panic!("expected retained level-zero pixels") };
    let expected = image::load_from_memory(&source(&bytes)).unwrap().into_rgba8();
    assert_eq!(pixels.as_slice(), expected.as_raw());
    assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[0] != p[1]));
}

#[test]
fn lazy_placed_images_keep_the_original_encoded_bytes() {
    let Some((bytes, doc, images)) = read_fixture("patchy-lazy-placed.af") else { return };
    assert!(doc.warnings.is_empty());
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].height), (400, 300));
    assert_eq!(images[0].transform, Affine::IDENTITY);
    assert_eq!(images[0].pixels, Pixels::Encoded(source(&bytes)));
}

#[test]
fn eight_and_sixteen_bit_channels_retain_the_same_pixel_pattern() {
    let Some((_, doc8, images8)) = read_fixture("patchy-rgba8.af") else { return };
    let (_, doc16, images16) = read_fixture("patchy-rgba16.af").unwrap();
    assert!(doc8.warnings.is_empty());
    assert!(doc16.warnings.is_empty());
    assert_eq!(images8.len(), 1);
    assert_eq!(images16, images8);
    assert_eq!((images8[0].width, images8[0].height), (64, 48));
    let Pixels::Rgba8(pixels) = &images8[0].pixels else { panic!() };
    assert_eq!(pixels.len(), 64 * 48 * 4);
    assert_eq!(&pixels[..4], &[0, 0, 0, 128]);
    assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] < 255));
    assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[0] != p[1]));
    // Frozen pixel-retention baseline, including every alpha sample. It is not a render oracle.
    assert_eq!(crc32fast::hash(pixels), 0xc8ee4dc1);
}

#[test]
fn rotated_scaled_pixels_keep_the_full_affine_transform() {
    let Some((_, doc, images)) = read_fixture("patchy-transform.af") else { return };
    assert!(doc.warnings.is_empty());
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].height), (80, 60));
    let expected = [1.1742158910592235, 0.4286222593193142, -0.4286222593193142, 1.1742158910592235, 60.0, 40.0];
    for (actual, expected) in images[0].transform.0.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-12);
    }
    let Pixels::Rgba8(pixels) = &images[0].pixels else { panic!() };
    assert_eq!(pixels.len(), 80 * 60 * 4);
    assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    assert_eq!(crc32fast::hash(pixels), 0x3720a442);
}

#[test]
fn cmyk_pixels_stay_cmyk_and_lab_losses_are_reported() {
    // CMYK layers keep their inks for the importer to convert through a colour profile.
    let Some((_, cmyk, images)) = read_fixture("patchy-cmyk.af") else { return };
    assert_eq!(images.len(), 1);
    assert_eq!((images[0].width, images[0].height), (64, 48));
    let Pixels::Cmyka8(pixels) = &images[0].pixels else { panic!() };
    assert_eq!(pixels.len(), 64 * 48 * 5);
    assert_eq!(crc32fast::hash(pixels), 0x56b1f659);
    assert!(cmyk.warnings.is_empty(), "{:?}", cmyk.warnings);
    assert_eq!(cmyk.cmyk_profile, None, "this file embeds no profile");
    let (_, lab, images) = read_fixture("patchy-lab.af").unwrap();
    assert!(images.is_empty());
    assert_eq!(lab.warnings, ["pixel layers in grey, Lab or 32-bit formats"]);
}
