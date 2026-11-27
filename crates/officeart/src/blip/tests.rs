#![allow(clippy::unwrap_used)]

use super::*;
use crate::record::build::*;

/// A metafile BLIP of `kind` and `instance` holding `bytes`, deflated when `deflate`, its header
/// giving `size` as the uncompressed size and the saved size of what follows.
fn metafile(kind: u16, instance: u16, size: usize, bytes: &[u8], deflate: bool) -> Vec<u8> {
    let saved = if deflate { miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6) } else { bytes.to_vec() };
    let mut header = vec![0u8; 34];
    header[..4].copy_from_slice(&(size as u32).to_le_bytes());
    header[28..32].copy_from_slice(&(saved.len() as u32).to_le_bytes());
    header[32] = if deflate { 0 } else { 0xFE };
    let uids = if instance & 1 == 1 { 32 } else { 16 };
    atom(kind, instance, &[&vec![0u8; uids][..], &header, &saved].concat())
}

/// An FBSE whose BLIP is `size` bytes at `offset` in the delay stream.
fn fbse_at(offset: u32, size: u32) -> Vec<u8> {
    let mut fbse = vec![0u8; 36];
    fbse[20..24].copy_from_slice(&size.to_le_bytes());
    fbse[28..32].copy_from_slice(&offset.to_le_bytes());
    atom(rt::FBSE, 3, &fbse)
}

fn store_of(entries: &[Vec<u8>], delay: &[u8], budget: usize) -> Store<'static> {
    let dgg = container(rt::DGG_CONTAINER, 0, &[container(rt::BSTORE_CONTAINER, entries.len() as u16, entries)]);
    let (dgg, delay): (&'static [u8], &'static [u8]) = (Box::leak(dgg.into_boxed_slice()), Box::leak(delay.to_vec().into_boxed_slice()));
    Store::new(Rec::at(dgg, 0), delay, budget)
}

fn picture_of(blip: &[u8], limit: usize) -> Option<Picture> {
    picture(&parse(&Rec::at(blip, 0).unwrap())?, limit, &mut Decode::new(limit))
}

/// Many entries pointing at one deflated metafile decode within the shared budget, each at most
/// once; one that decodes costs its compressed bytes and what they inflated to.
#[test]
fn the_store_decodes_lazily_within_a_budget() {
    let wmf = vec![7u8; 1000];
    let blip = metafile(rt::BLIP_WMF, 0x216, 1000, &wmf, true);
    let input = blip.len() - 8 - 16 - 34;
    let entries: Vec<Vec<u8>> = (0..5).map(|_| fbse_at(0, blip.len() as u32)).collect();
    let mut store = store_of(&entries, &blip, 2 * (input + 1000) + 10);
    assert_eq!(store.take(0).map(|p| p.data.len()), Some(1000));
    assert_eq!(store.left(), input + 1000 + 10);
    assert!(store.take(0).is_none(), "taken already");
    assert!(store.take(1).is_some());
    assert!(store.take(2).is_none(), "past the budget");
    assert!(store.take(99).is_none());
}

/// A compressed metafile that fails to inflate still costs the budget its compressed bytes and
/// what it may have inflated, so entries pointing at one damaged stream can't each inflate it
/// again for free; once that no longer fits, it isn't inflated at all.
#[test]
fn a_failed_inflate_is_charged_to_the_budget() {
    let mut damaged = metafile(rt::BLIP_WMF, 0x216, 1000, &[], false);
    // Deflated, its stream damaged: four bytes.
    let at = 8 + 16;
    damaged[at + 28..at + 32].copy_from_slice(&4u32.to_le_bytes());
    damaged[at + 32] = 0;
    damaged.extend_from_slice(&[0x78, 0x9C, 0xFF, 0xFF]);
    let n = (damaged.len() - 8) as u32;
    damaged[4..8].copy_from_slice(&n.to_le_bytes());
    let entries: Vec<Vec<u8>> = (0..3).map(|_| fbse_at(0, damaged.len() as u32)).collect();
    let mut store = store_of(&entries, &damaged, 2500);
    assert!(store.take(0).is_none());
    assert_eq!(store.left(), 1496, "1000 bytes it may have inflated and the 4 it read");
    assert!(store.take(1).is_none());
    assert_eq!(store.left(), 492);
    assert!(store.take(2).is_none());
    assert_eq!(store.left(), 492, "too little left: refused before inflating");
    // A picture that can't be read costs nothing.
    assert!(store.decode_inline(&atom(rt::BLIP_PNG, 0x6E0, &[0; 4])).is_none());
    assert_eq!(store.left(), 492);
}

/// A compressed metafile whose header gives no uncompressed size inflates to nothing: no picture,
/// and nothing charged.
#[test]
fn a_compressed_metafile_without_a_size_is_no_picture() {
    let blip = metafile(rt::BLIP_WMF, 0x216, 0, &[7; 100], true);
    let mut work = Decode::new(1000);
    assert!(picture(&parse(&Rec::at(&blip, 0).unwrap()).unwrap(), 1000, &mut work).is_none());
    assert_eq!(work.left, 1000);
}

#[test]
fn decodes_png_and_deflated_metafiles() {
    let png = atom(rt::BLIP_PNG, 0x6E0, &[&[0u8; 17][..], b"\x89PNG..."].concat());
    let p = picture_of(&png, usize::MAX).unwrap();
    assert_eq!((p.ext, &p.data[..4]), ("png", &b"\x89PNG"[..]));
    // Over the remaining budget: refused before it is copied.
    assert!(picture_of(&png, 6).is_none());

    let wmf = b"\x01\x00\x09\x00\x00\x03 a tiny metafile".to_vec();
    let p = picture_of(&metafile(rt::BLIP_WMF, 0x216, wmf.len(), &wmf, true), usize::MAX).unwrap();
    assert_eq!((p.ext, p.data), ("wmf", wmf.clone()));
    // Damaged deflate data is no picture rather than a panic.
    let mut bad = metafile(rt::BLIP_EMF, 0x3D4, wmf.len(), &wmf, true);
    let n = bad.len();
    bad[n - 6..].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
    assert!(picture_of(&bad, usize::MAX).is_none());
}

#[test]
fn a_dib_becomes_a_bmp_the_image_crate_reads() {
    // 1×1, 24-bit.
    let mut dib = vec![0u8; 40];
    dib[0] = 40;
    dib[4] = 1;
    dib[8] = 1;
    dib[12] = 1;
    dib[14] = 24;
    dib.extend_from_slice(&[0, 0, 255, 0]);
    let p = picture_of(&atom(rt::BLIP_DIB, 0x7A8, &[&[0u8; 17][..], &dib].concat()), usize::MAX).unwrap();
    assert_eq!((p.ext, &p.data[..2]), ("bmp", &b"BM"[..]));
    assert_eq!(u32::from_le_bytes(p.data[10..14].try_into().unwrap()), 54);
    let img = image::load_from_memory_with_format(&p.data, image::ImageFormat::Bmp).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (1, 1));
    assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
}

/// Every copy a DIB makes is charged before it is made: its whole BMP file (14 bytes more than
/// the DIB), then for a PNG the decoded pixels and the PNG itself.
#[test]
fn every_copy_of_a_dib_is_charged() {
    // 1×1, 24-bit: 4 bytes of decoded pixels.
    let mut dib = vec![0u8; 40];
    dib[0] = 40;
    dib[4] = 1;
    dib[8] = 1;
    dib[12] = 1;
    dib[14] = 24;
    dib.extend_from_slice(&[0, 0, 255, 0]);
    let blip = atom(rt::BLIP_DIB, 0x7A8, &[&[0u8; 17][..], &dib].concat());
    let bmp = 14 + dib.len();
    let read = |left: usize| picture(&parse(&Rec::at(&blip, 0).unwrap()).unwrap(), usize::MAX, &mut Decode::new(left));
    assert!(read(dib.len()).is_none(), "the DIB fits, its BMP doesn't");
    assert!(read(bmp).is_some());
    // The BMP and the pixels fit, the PNG doesn't: the BMP is kept.
    let mut work = Decode::new(bmp + 4);
    assert!(matches!(crate::dib::to_png(&dib, usize::MAX, &mut work), Some(crate::dib::Dib::Bmp(_))));
    let mut work = Decode::new(1 << 20);
    let Some(crate::dib::Dib::Png(png)) = crate::dib::to_png(&dib, usize::MAX, &mut work) else { panic!("not a PNG") };
    assert!((1 << 20) - work.left >= bmp + 4 + png.len(), "{} charged", (1 << 20) - work.left);
}

/// A DIB too wide to be decoded is kept as a BMP, charged only its copy, not pixels it never
/// decodes, so the budget is left for later pictures.
#[test]
fn a_dib_too_large_to_decode_is_not_charged_its_pixels() {
    // 20,000 × 1, 1-bit, with a two-colour table and one row of pixels.
    let mut dib = vec![0u8; 40];
    dib[0] = 40;
    dib[4..8].copy_from_slice(&20_000u32.to_le_bytes());
    dib[8] = 1;
    dib[12] = 1;
    dib[14] = 1;
    dib.extend_from_slice(&[0; 8]);
    dib.extend_from_slice(&[0; 2500]);
    let bmp = 14 + dib.len();
    let mut work = Decode::new(1 << 20);
    assert!(matches!(crate::dib::to_png(&dib, usize::MAX, &mut work), Some(crate::dib::Dib::Bmp(_))));
    assert_eq!((1 << 20) - work.left, bmp, "only the BMP copy");
}

/// A DIB whose layout isn't a bitmap's (here an 8-bit one without room for its colour table) is
/// refused before its copy is charged, so it costs later pictures nothing.
#[test]
fn a_dib_that_isnt_a_bitmap_is_refused_before_it_is_charged() {
    let mut dib = vec![0u8; 40];
    dib[0] = 40;
    dib[4] = 1;
    dib[8] = 1;
    dib[12] = 1;
    dib[14] = 8;
    let mut work = Decode::new(1 << 20);
    assert!(crate::dib::to_png(&dib, usize::MAX, &mut work).is_none());
    assert_eq!(work.left, 1 << 20);
}

/// A WMF's file with its placeable header is a second copy of its records, charged whole with the
/// header; when that doesn't fit in the limit or the budget, or its bounds can't make a header,
/// the records are kept as a WMF without one.
#[test]
fn a_wmf_header_is_added_only_when_its_copy_fits() {
    let records = vec![3u8; 1024];
    let wmf = |l: i32, r: i32| {
        let mut b = metafile(rt::BLIP_WMF, 0x216, records.len(), &records, false);
        // The bounds, at 16 + 4 (the UID, then the uncompressed size) in the record body.
        b[8 + 20..8 + 24].copy_from_slice(&l.to_le_bytes());
        b[8 + 28..8 + 32].copy_from_slice(&r.to_le_bytes());
        b
    };
    let fits = wmf(0, 1000);
    let blip = parse(&Rec::at(&fits, 0).unwrap()).unwrap();
    let whole = records.len() + crate::wmf::PLACEABLE_HEADER;
    let read = |limit: usize, left: usize| {
        let mut work = Decode::new(left);
        with_placeable_header(&blip, limit, &mut work).map(|b| (b.len(), left - work.left))
    };
    assert_eq!(read(usize::MAX, records.len() + whole), Some((whole, records.len() + whole)), "both copies charged");
    assert_eq!(read(usize::MAX, whole), Some((records.len(), records.len())), "the second copy doesn't fit: kept bare");
    assert_eq!(read(whole - 1, usize::MAX), Some((records.len(), records.len())), "the file doesn't fit the limit: kept bare");
    let wide = wmf(40_000, 41_000);
    let blip = parse(&Rec::at(&wide, 0).unwrap()).unwrap();
    let mut work = Decode::new(records.len());
    assert_eq!(with_placeable_header(&blip, records.len(), &mut work).map(|b| b.len()), Some(records.len()), "no header to add, none reserved");
}

/// A WMF whose bounds don't fit the 16-bit coordinates of its records keeps its records without a
/// placeable header, rather than one whose clamped bounds have no width.
#[test]
fn a_wmf_with_bounds_past_16_bits_has_no_placeable_header() {
    let records = b"WMF records".to_vec();
    let header = |l: i32, r: i32| {
        let mut h = Vec::new();
        for v in [records.len() as i32, l, 0, r, 1000, 914_400, 914_400, records.len() as i32] {
            h.extend_from_slice(&v.to_le_bytes());
        }
        h.extend_from_slice(&[0xFE, 0xFE]);
        h
    };
    assert_eq!(crate::wmf::with_placeable_header(&header(40_000, 41_000), records.clone()), Some(records.clone()));
    let with = crate::wmf::with_placeable_header(&header(0, 1000), records.clone()).unwrap();
    assert_eq!((with.len(), &with[22..]), (records.len() + crate::wmf::PLACEABLE_HEADER, &records[..]));
}

/// The pixels start after the colour table: biClrUsed entries for more than 8 bits (none when it
/// is 0), at most 2^bits for fewer, after the masks of BI_BITFIELDS (3) or BI_ALPHABITFIELDS (4);
/// a table running past the DIB is no picture.
#[test]
fn a_dib_colour_table_is_sized_by_its_bit_depth() {
    let dib = |bits: u16, compression: u32, used: u32, extra: usize| {
        let mut d = vec![0u8; 40];
        d[0] = 40;
        d[14..16].copy_from_slice(&bits.to_le_bytes());
        d[16..20].copy_from_slice(&compression.to_le_bytes());
        d[32..36].copy_from_slice(&used.to_le_bytes());
        d.resize(40 + extra, 0);
        d
    };
    let offset = |b: &[u8]| dib::to_bmp(b).map(|bmp| u32::from_le_bytes(bmp[10..14].try_into().unwrap()));
    assert_eq!(offset(&dib(24, 0, 0, 4)), Some(54));
    assert_eq!(offset(&dib(24, 0, 300, 1204)), Some(54 + 1200), "more than 256 entries for 24 bits");
    assert_eq!(offset(&dib(8, 0, 300, 1028)), Some(54 + 1024), "8 bits: at most 256");
    assert_eq!(offset(&dib(4, 0, 0, 68)), Some(54 + 64));
    assert_eq!(offset(&dib(32, 3, 0, 16)), Some(54 + 12), "three masks");
    assert_eq!(offset(&dib(32, 6, 0, 16)), Some(54 + 16), "four masks");
    assert_eq!(offset(&dib(24, 0, 300, 100)), None, "the table runs past the DIB");
    assert_eq!(offset(&dib(32, 0, u32::MAX, 0)), None);
    // A header size no Windows version writes.
    let mut odd = dib(24, 0, 0, 4);
    odd[0] = 200;
    assert_eq!(offset(&odd), None);
}

/// JPEG (RGB and CMYK), TIFF and a PNG with a second UID: the instance says how many UIDs come
/// before the tag byte, and the bytes after it are the file.
#[test]
fn decodes_each_bitmap_kind() {
    let file = b"the picture file";
    for (kind, instance, uids, ext) in [
        (rt::BLIP_JPEG, 0x46A, 16, "jpg"),
        (rt::BLIP_JPEG, 0x46B, 32, "jpg"),
        (rt::BLIP_JPEG_CMYK, 0x6E2, 16, "jpg"),
        (rt::BLIP_TIFF, 0x6E4, 16, "tif"),
        (rt::BLIP_TIFF, 0x6E5, 32, "tif"),
        (rt::BLIP_PNG, 0x6E1, 32, "png"),
    ] {
        let body = [&vec![0xAAu8; uids][..], &[0xFF], file].concat();
        let p = picture_of(&atom(kind, instance, &body), usize::MAX).unwrap();
        assert_eq!((p.ext, p.data.as_slice()), (ext, &file[..]), "{kind:#x}/{instance:#x}");
    }
}

/// Only the two instances [MS-ODRAW] gives each type are read, and a metafile is deflated (0) or
/// stored (0xFE): anything else is no picture.
#[test]
fn an_unknown_instance_or_compression_is_no_picture() {
    let body = [&[0u8; 17][..], b"\x89PNG"].concat();
    assert!(parse(&Rec::at(&atom(rt::BLIP_PNG, 0x6E0, &body), 0).unwrap()).is_some());
    assert!(parse(&Rec::at(&atom(rt::BLIP_PNG, 0x6E2, &body), 0).unwrap()).is_none());
    assert!(parse(&Rec::at(&atom(0xF018, 0, &body), 0).unwrap()).is_none(), "not a picture type");
    let mut odd = metafile(rt::BLIP_EMF, 0x3D4, 4, b"EMF!", false);
    odd[8 + 16 + 32] = 0x01;
    assert!(parse(&Rec::at(&odd, 0).unwrap()).is_none());
}

/// A metafile stored without compression is copied up to its stated size and saved size, whatever
/// follows; with two UIDs the header starts 32 bytes in.
#[test]
fn decodes_an_uncompressed_metafile() {
    let emf = b"\x01\x00\x00\x00 an enhanced metafile".to_vec();
    for instance in [0x3D4, 0x3D5] {
        let mut blip = metafile(rt::BLIP_EMF, instance, emf.len(), &emf, false);
        blip.extend_from_slice(b"trailing");
        let n = (blip.len() - 8) as u32;
        blip[4..8].copy_from_slice(&n.to_le_bytes());
        let p = picture_of(&blip, usize::MAX).unwrap();
        assert_eq!((p.ext, &p.data), ("emf", &emf));
    }
}

/// A metafile whose inflated size (with its compressed bytes) would pass the remaining budget is
/// refused, compressed or not.
#[test]
fn a_metafile_over_the_budget_is_refused() {
    let wmf = vec![9u8; 4000];
    let deflated = metafile(rt::BLIP_WMF, 0x216, 4000, &wmf, true);
    let input = deflated.len() - 8 - 16 - 34;
    // The budget covers the compressed bytes read as well as the bytes they inflate to.
    assert!(picture_of(&deflated, 3999 + input).is_none());
    assert_eq!(picture_of(&deflated, 4000 + input).map(|p| p.data.len()), Some(4000));
    let raw = metafile(rt::BLIP_WMF, 0x216, 4000, &wmf, false);
    assert!(picture_of(&raw, 3999).is_none());
    assert!(picture_of(&raw, 4000).is_some());
}

/// An FBSE may hold its BLIP itself, after its name, instead of pointing into the delay stream;
/// one whose size is 0 holds no picture.
#[test]
fn a_blip_inside_its_fbse_is_read() {
    let blip = atom(rt::BLIP_PNG, 0x6E0, &[&[0u8; 17][..], b"\x89PNG inside"].concat());
    let name = "p".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
    let entry = |size: u32| {
        let mut fbse = [0u8; 36];
        fbse[20..24].copy_from_slice(&size.to_le_bytes());
        fbse[33] = name.len() as u8;
        atom(rt::FBSE, 6, &[&fbse[..], &name, &blip].concat())
    };
    // The delay stream is empty: the BLIP can only come from the FBSE.
    let mut store = store_of(&[entry(blip.len() as u32), entry(0)], &[], usize::MAX);
    let p = store.take(0).unwrap();
    assert_eq!((p.ext, &p.data[..]), ("png", &b"\x89PNG inside"[..]));
    assert!(store.take(1).is_none(), "an entry of size 0");
}

/// A store child that isn't an FBSE (a BLIP stored directly) still takes a slot, so the `pib`s
/// of the entries after it don't shift onto the wrong picture.
#[test]
fn a_child_that_isnt_an_fbse_keeps_its_slot() {
    let png = |tag: &[u8]| atom(rt::BLIP_PNG, 0x6E0, &[&[0u8; 17][..], b"\x89PNG", tag].concat());
    let (first, second) = (png(b"1"), png(b"2"));
    let delay = [&first[..], &second[..]].concat();
    let entries = [fbse_at(0, first.len() as u32), png(b"direct"), fbse_at(first.len() as u32, second.len() as u32)];
    let mut store = store_of(&entries, &delay, usize::MAX);
    assert!(store.take(1).is_none(), "the direct BLIP isn't decoded here");
    assert_eq!(store.take(2).map(|p| p.data), Some(b"\x89PNG2".to_vec()));
    assert_eq!(store.take(0).map(|p| p.data), Some(b"\x89PNG1".to_vec()));
}

/// An FBSE with bytes after its name holds its BLIP there; when those bytes aren't a record, the
/// entry is no picture, rather than whatever its `foDelay` points at in the delay stream.
#[test]
fn an_unreadable_embedded_blip_is_not_looked_for_in_the_delay_stream() {
    let blip = atom(rt::BLIP_PNG, 0x6E0, &[&[0u8; 17][..], b"\x89PNG"].concat());
    let mut fbse = [0u8; 36];
    fbse[20..24].copy_from_slice(&(blip.len() as u32).to_le_bytes());
    let entry = atom(rt::FBSE, 2, &[&fbse[..], &[1, 2, 3]].concat());
    let mut store = store_of(&[entry, fbse_at(0, blip.len() as u32)], &blip, usize::MAX);
    assert!(store.take(0).is_none());
    assert!(store.take(1).is_some(), "the same delay stream offset, pointed at without embedded bytes");
}

/// The store reads at most [`MAX_ENTRIES`] entries.
#[test]
fn the_store_holds_at_most_max_entries() {
    let png = atom(rt::BLIP_PNG, 0x6E0, &[&[0u8; 17][..], b"\x89PNG"].concat());
    let entries: Vec<Vec<u8>> = (0..MAX_ENTRIES + 10).map(|_| fbse_at(0, png.len() as u32)).collect();
    let mut store = store_of(&entries, &png, usize::MAX);
    assert!(store.take(MAX_ENTRIES - 1).is_some());
    assert!(store.take(MAX_ENTRIES).is_none());
}
