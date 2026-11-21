//! The JPEG encoder writes `Metadata::photoshop_resources` as an APP13 `Photoshop 3.0` segment,
//! in the exact shape its own decoder reads (marker 0xED, the identifier, then the `8BIM`
//! resource blocks verbatim). A block too large for one segment is split over consecutive
//! APP13 segments, each carrying the identifier again: that split matches how `scan_metadata`
//! concatenates the segments back, not Photoshop's own continuation format, whose follow-up
//! segments repeat neither the identifier nor `8BIM` (exiv2 bug #533). A block past the 16 MiB
//! bound `scan_metadata` reads is left out whole, like metadata too large for its EXIF/XMP
//! segment. No resources means no segment.

use crate::common;
use common::*;
use craft_image_codecs::*;

/// One Photoshop image resource as its own bytes: `8BIM`, big-endian id, empty Pascal name
/// (padded to an even length), big-endian data size, and the data (padded to an even length).
fn resource(id: u16, data: &[u8]) -> Vec<u8> {
    let mut r = b"8BIM".to_vec();
    r.extend_from_slice(&id.to_be_bytes());
    r.extend_from_slice(&[0, 0]); // "" name, one length byte padded to even
    r.extend_from_slice(&(data.len() as u32).to_be_bytes());
    r.extend_from_slice(data);
    if data.len() % 2 == 1 {
        r.push(0);
    }
    r
}

/// A Color Samplers (1073) shaped block: one point per entry, each 16.16 fixed pixels.
fn samplers(entries: &[(i16, i16)]) -> Vec<u8> {
    let mut points = Vec::new();
    for &(x, y) in entries {
        points.extend_from_slice(&(i32::from(x) << 16).to_be_bytes());
        points.extend_from_slice(&(i32::from(y) << 16).to_be_bytes());
    }
    resource(1073, &points)
}

/// Every APP13 payload in the file, walking the marker segments up to the first SOS.
fn app13_payloads(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 2;
    while i + 4 <= bytes.len() && bytes[i] == 0xFF {
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            break;
        }
        if marker == 0xED {
            out.push(&bytes[i + 4..i + 2 + len]);
        }
        i += 2 + len;
    }
    out
}

#[test]
fn photoshop_resources_round_trip_through_jpeg() {
    for layout in [ChannelLayout::Rgb, ChannelLayout::Gray] {
        let mut img = synth(9, 7, layout, SampleType::U8, 5, 0.3);
        let resources = [samplers(&[(199, 149), (12, 34)]), resource(1039, b"\x00\x01")].concat();
        img.meta.photoshop_resources = Some(resources.clone());
        let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.meta.photoshop_resources.as_deref(), Some(resources.as_slice()), "{layout:?}");
    }
}

#[test]
fn empty_resources_write_no_app13_segment() {
    for resources in [None, Some(Vec::new())] {
        let mut img = synth(9, 7, ChannelLayout::Rgb, SampleType::U8, 5, 0.3);
        img.meta.photoshop_resources = resources;
        let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        assert!(app13_payloads(&bytes).is_empty());
        assert_eq!(decode(&bytes).unwrap().meta.photoshop_resources, None);
    }
}

#[test]
fn app13_uses_the_photoshop_identifier() {
    let mut img = synth(9, 7, ChannelLayout::Rgb, SampleType::U8, 5, 0.3);
    img.meta.photoshop_resources = Some(samplers(&[(7, 9)]));
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let payloads = app13_payloads(&bytes);
    assert_eq!(payloads.len(), 1);
    assert!(payloads[0].starts_with(b"Photoshop 3.0\0"));
}

#[test]
fn a_large_block_splits_over_app13_segments_matching_scan_metadata() {
    let mut img = synth(9, 7, ChannelLayout::Rgb, SampleType::U8, 5, 0.3);
    // 5000 bytes: over two 2048-byte chunks, under the 65519 a single segment could hold.
    let resources = resource(1073, &vec![0x5A; 5000]);
    img.meta.photoshop_resources = Some(resources.clone());
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let payloads = app13_payloads(&bytes);
    assert_eq!(payloads.len(), 3);
    for p in &payloads {
        assert!(p.starts_with(b"Photoshop 3.0\0"));
        assert!(p.len() <= 14 + 2048);
    }
    let back = decode(&bytes).unwrap();
    assert_eq!(back.meta.photoshop_resources.as_deref(), Some(resources.as_slice()));
}

/// The block is document content, not metadata: like the TIFF writer, the JPEG encoder keeps
/// it when `embed_metadata` is off, and the decoder reads it back.
#[test]
fn resources_survive_embed_metadata_off() {
    let mut img = synth(9, 7, ChannelLayout::Rgb, SampleType::U8, 5, 0.3);
    let resources = samplers(&[(199, 149)]);
    img.meta.photoshop_resources = Some(resources.clone());
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions { embed_metadata: false, ..Default::default() }).unwrap();
    assert_eq!(app13_payloads(&bytes).len(), 1);
    assert_eq!(decode(&bytes).unwrap().meta.photoshop_resources.as_deref(), Some(resources.as_slice()));
}

/// The resources are opaque bytes: a block that itself contains marker-shaped bytes must
/// survive intact, and must not be mistaken for the file's own structure on the way back in.
#[test]
fn marker_shaped_resource_data_survives() {
    let mut img = synth(9, 7, ChannelLayout::Rgb, SampleType::U8, 5, 0.3);
    let hostile = [0xFFu8, 0xD9, 0x00, 0xFF, 0xDA, 0x41, 0xFF, 0x00].repeat(300);
    let resources = resource(1073, &hostile);
    img.meta.photoshop_resources = Some(resources.clone());
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back.meta.photoshop_resources.as_deref(), Some(resources.as_slice()));
}

/// The split boundary pinned exactly at `PHOTOSHOP_APP13_CHUNK` (2048): a block of exactly
/// 2048 resource bytes stays in one segment, one byte more splits into a full 2048-byte
/// segment plus a 1-byte segment; both sizes decode back intact.
#[test]
fn app13_split_boundary_is_exactly_2048_resource_bytes() {
    for (total, segments) in [(2048usize, 1usize), (2049, 2)] {
        let mut img = synth(9, 7, ChannelLayout::Rgb, SampleType::U8, 5, 0.3);
        let mut resources = resource(1073, &vec![0x5A; total - 12]);
        if resources.len() != total {
            resources.pop(); // `resource` pads an odd body to even; drop the pad for the odd total
        }
        assert_eq!(resources.len(), total);
        img.meta.photoshop_resources = Some(resources.clone());
        let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        let payloads = app13_payloads(&bytes);
        assert_eq!(payloads.len(), segments, "{total} resource bytes");
        for p in &payloads {
            assert!(p.starts_with(b"Photoshop 3.0\0"), "{total} resource bytes");
        }
        assert_eq!(payloads[0].len(), 14 + total.min(2048), "{total} resource bytes");
        if segments == 2 {
            assert_eq!(payloads[1].len(), 14 + 1, "{total} resource bytes");
        }
        let back = decode(&bytes).unwrap();
        assert_eq!(back.meta.photoshop_resources.as_deref(), Some(resources.as_slice()), "{total} resource bytes");
    }
}

/// The decoder side without our encoder's APP13 code: one resource block spread by hand over
/// two APP13 segments, each carrying its own identifier, split at an odd offset no chunk of
/// our encoder would produce. The segments are spliced into an ordinary encoded JPEG whose
/// image itself carries no resources; `scan_metadata` must still rejoin the block verbatim.
#[test]
fn hand_formed_split_app13_segments_decode_back_joined() {
    let img = synth(9, 7, ChannelLayout::Rgb, SampleType::U8, 5, 0.3);
    let base = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let resources = [samplers(&[(5, 6)]), resource(1039, &vec![0x21; 400])].concat();
    let app13 = |part: &[u8]| -> Vec<u8> {
        let payload = [b"Photoshop 3.0\0".as_slice(), part].concat();
        let mut s = vec![0xFF, 0xED];
        s.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        s.extend_from_slice(&payload);
        s
    };
    let split = 137; // odd, and far from the encoder's 2048-byte chunk size
    let mut i = 2;
    while (0xE0..=0xEF).contains(&base[i + 1]) {
        i += 2 + u16::from_be_bytes([base[i + 2], base[i + 3]]) as usize;
    }
    let mut bytes = base[..i].to_vec();
    bytes.extend_from_slice(&app13(&resources[..split]));
    bytes.extend_from_slice(&app13(&resources[split..]));
    bytes.extend_from_slice(&base[i..]);
    let payloads = app13_payloads(&bytes);
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[0].len(), 14 + split);
    assert_eq!(payloads[1].len(), 14 + (resources.len() - split));
    let back = decode(&bytes).unwrap();
    assert_eq!(back.meta.photoshop_resources.as_deref(), Some(resources.as_slice()));
}

/// `scan_metadata` stops collecting APP13 payloads at 16 MiB (jpeg.rs `1 << 24`), so a larger
/// block would be read back silently truncated. Such a block is left out whole, the way
/// metadata too large for its one EXIF/XMP segment is left out; the bound itself is written
/// and comes back exact — 8192 full 2048-byte chunks fit inside it.
#[test]
fn resources_past_the_scanner_bound_are_left_out_whole() {
    for (len, written) in [(1usize << 24, true), ((1usize << 24) + 1, false)] {
        let mut img = synth(9, 7, ChannelLayout::Rgb, SampleType::U8, 5, 0.3);
        let mut resources = resource(1073, &vec![0x5A; len - 12]);
        if resources.len() != len {
            resources.pop(); // reach the odd length exactly, past `resource`'s even padding
        }
        assert_eq!(resources.len(), len);
        img.meta.photoshop_resources = Some(resources.clone());
        let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
        assert_eq!(!app13_payloads(&bytes).is_empty(), written, "{len} resource bytes");
        let back = decode(&bytes).unwrap();
        let want = if written { Some(resources.as_slice()) } else { None };
        assert_eq!(back.meta.photoshop_resources.as_deref(), want, "{len} resource bytes");
    }
}
