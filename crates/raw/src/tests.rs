use super::*;

#[test]
fn cfa_helpers() {
    let c = Cfa::bayer("RGGB").unwrap();
    assert!(c.is_bayer());
    assert_eq!(c.color_at(0, 0), 0);
    assert_eq!(c.color_at(3, 3), 2);
    assert_eq!(c.shifted(1, 0).name(), "GRBG");
    assert_eq!(c.shifted(1, 1).name(), "BGGR");
    assert!(Cfa::bayer("RGGX").is_none());
    for name in ["RGGB", "BGGR", "GRBG", "GBRG"] {
        assert_eq!(Cfa::bayer_static(name).name(), name);
    }
    let x = Cfa::xtrans();
    assert!(!x.is_bayer());
    let greens = x.pattern.iter().filter(|&&p| p == 1).count();
    assert_eq!(greens, 20);
    // every 3×3 block of the X-Trans layout has 5 greens
    for by in 0..2 {
        for bx in 0..2 {
            let g = (0..9).filter(|i| x.color_at(bx * 3 + i % 3, by * 3 + i / 3) == 1).count();
            assert_eq!(g, 5);
        }
    }
}

#[test]
fn an_active_area_that_wraps_past_the_image_is_corrupt() {
    let (w, h) = (12usize, 10usize);
    let mut raw = RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16(vec![100; w * h]),
        cfa: Cfa::bayer("RGGB"),
        bits: 12,
        black: BlackLevel::uniform(64.0),
        white: vec![1023.0],
        active_area: Rect::new(usize::MAX, 0, 1, 1),
        crop: Rect::default(),
        orientation: Orientation::Normal,
        color: ColorData::default(),
        wb_multipliers: None,
        linearized: false,
        opcodes: OpcodeLists::default(),
        camera: Camera::default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
        pixel_aspect: 1.0,
    };
    assert!(raw.validate().is_err(), "`x + width` wrapped: the area is outside the image");
    raw.active_area = Rect::new(0, 0, w + 1, h);
    assert!(raw.validate().is_err(), "one past the last column is outside too");
}

#[test]
fn black_levels() {
    let b = BlackLevel { repeat_rows: 2, repeat_cols: 2, values: vec![1.0, 2.0, 3.0, 4.0], delta_h: vec![0.5, 0.0], delta_v: vec![0.0, 10.0] };
    assert_eq!(b.at(0, 0, 0, 1), 1.5);
    assert_eq!(b.at(1, 1, 0, 1), 14.0);
    assert_eq!(b.at(3, 2, 0, 1), 2.0);
    assert_eq!(b.mean(), 2.5);
    assert_eq!(BlackLevel::uniform(7.0).at(9, 9, 0, 3), 7.0);
}

// --- TIFF shells around a private raw block (thumbnail-only IFD0) ---

use craft_tiff::{IfdBuilder, ImageData, TiffWriter, Value, tags as t};

/// An uncompressed 8-bit RGB image IFD.
fn rgb_ifd(w: u32, h: u32) -> IfdBuilder {
    let mut ifd = IfdBuilder::new();
    ifd.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
    ifd.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
    ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![8, 8, 8]));
    ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
    ifd.set(t::PHOTOMETRIC, Value::Short(vec![2]));
    ifd.set(t::COMPRESSION, Value::Short(vec![1]));
    ifd.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![vec![90u8; (w * h * 3) as usize]] });
    ifd
}

fn exif_size(w: u32, h: u32) -> IfdBuilder {
    IfdBuilder::new().with(t::PIXEL_X_DIMENSION, Value::Long(vec![w])).with(t::PIXEL_Y_DIMENSION, Value::Long(vec![h]))
}

fn write(chain: &[IfdBuilder]) -> Vec<u8> {
    TiffWriter::default().write(chain).unwrap()
}

const SHELL_REASON: &str = "raw image in a private block";

/// A bigger image in a `SubIFDs` child of a small IFD0 (the layout of an 8-bit-mosaic TIFF raw).
#[test]
fn thumbnail_with_a_bigger_sub_ifd_is_an_undecodable_raw() {
    let mut ifd0 = rgb_ifd(16, 12);
    ifd0.add_sub_ifd(rgb_ifd(64, 48));
    let bytes = write(&[ifd0]);
    assert_eq!(probe(&bytes), Some(RawFormat::OtherTiff));
    let Err(RawError::Unsupported(why)) = decode(&bytes) else { panic!("expected Unsupported") };
    assert!(why.contains("64x48") && why.contains("16x12") && why.contains(SHELL_REASON), "{why}");
    // a sub-IFD that is not much bigger (a pyramid level) does not count
    let mut ifd0 = rgb_ifd(16, 12);
    ifd0.add_sub_ifd(rgb_ifd(32, 24));
    assert_eq!(probe(&write(&[ifd0])), None);
}

/// Exif says the picture is far bigger than IFD0 and the file holds a block that could carry it.
#[test]
fn thumbnail_with_a_private_block_behind_the_exif_size_is_an_undecodable_raw() {
    let mut ifd0 = rgb_ifd(16, 12);
    ifd0.set_child(t::EXIF_IFD, exif_size(400, 300));
    let mut bytes = write(&[ifd0.clone()]);
    // no private block: a downsized copy that kept its old Exif size is an ordinary image
    assert_eq!(probe(&bytes), None);
    // 400 x 300 pixels at one bit each is 15000 bytes: 14000 is not enough, 16000 is
    bytes.extend(std::iter::repeat_n(7u8, 14_000));
    assert_eq!(probe(&bytes), None);
    bytes.extend(std::iter::repeat_n(7u8, 2_000));
    assert_eq!(probe(&bytes), Some(RawFormat::OtherTiff));
    let Err(RawError::Unsupported(why)) = decode(&bytes) else { panic!("expected Unsupported") };
    assert!(why.contains("400x300") && why.contains("16x12") && why.contains(SHELL_REASON), "{why}");
    // truncating it never panics
    for n in (0..bytes.len()).step_by(37) {
        let _ = probe(&bytes[..n]);
        let _ = decode(&bytes[..n]);
    }
}

/// Ordinary TIFFs stay images: the full picture first, equal Exif size, or later pages that are bigger.
#[test]
fn ordinary_tiffs_are_not_shells() {
    let padded = |chain: &[IfdBuilder]| {
        let mut b = write(chain);
        b.extend(std::iter::repeat_n(0u8, 100_000));
        b
    };
    let mut full = rgb_ifd(400, 300);
    full.set_child(t::EXIF_IFD, exif_size(400, 300));
    assert_eq!(probe(&padded(&[full])), None);
    // the Exif size is much bigger than IFD0, with spare bytes, but IFD0 is not tiny next to it
    let mut near = rgb_ifd(120, 90);
    near.set_child(t::EXIF_IFD, exif_size(400, 300));
    assert_eq!(probe(&padded(&[near])), None);
    // a multi-page TIFF whose first page is small
    assert_eq!(probe(&padded(&[rgb_ifd(16, 12), rgb_ifd(400, 300)])), None);
    // no dimensions at all
    assert_eq!(probe(&padded(&[IfdBuilder::new().with(t::MAKE, Value::Ascii("X".into()))])), None);
    // raw evidence without a CFA photometric: Pentax's Exif `CFAPattern` (uncompressed / PackBits PEFs) and
    // Samsung's private compressions (here 32773, PackBits elsewhere: NX1, NX500)
    let mut pef = rgb_ifd(400, 300);
    pef.set(t::MAKE, Value::Ascii("PENTAX Corporation".into()));
    let mut exif = exif_size(400, 300);
    exif.set(EXIF_CFA_PATTERN, Value::Undefined(vec![0, 2, 0, 2, 0, 1, 1, 2]));
    pef.set_child(t::EXIF_IFD, exif);
    assert_eq!(probe(&padded(&[pef])), Some(RawFormat::Pef));
    let mut srw = rgb_ifd(400, 300);
    srw.set(t::MAKE, Value::Ascii("SAMSUNG".into()));
    srw.set(t::COMPRESSION, Value::Short(vec![32773]));
    assert_eq!(probe(&padded(&[srw])), Some(RawFormat::Srw));
    // camera-authored or exported TIFFs preserving camera Make tags remain ordinary images
    for make in ["SONY", "NIKON CORPORATION", "PENTAX", "RICOH", "SAMSUNG"] {
        let mut cam = rgb_ifd(400, 300);
        cam.set(t::MAKE, Value::Ascii(make.into()));
        cam.set_child(t::EXIF_IFD, exif_size(400, 300));
        assert_eq!(probe(&padded(&[cam])), None, "{make} TIFF should not be probed as raw");
    }
}

/// storytold/lightcraft#702: two Sony layouts carry no CFA IFD, so the raw evidence storytold/lightcraft#281 asks of Sony files has to
/// include them: SRF (an RGB-looking IFD0 of 14-bit samples) and the DSLR-A100 (raw data addressed
/// by `SubIFDs`). An 8-bit TIFF from the same bodies, or a SubIFDs TIFF from another Sony model,
/// stays an ordinary image.
#[test]
fn sony_raws_without_a_cfa_ifd_are_still_raws() {
    let sony = |model: &str, bits: u16, compression: u16, sub_ifd: Option<IfdBuilder>| {
        let mut ifd = rgb_ifd(400, 300);
        ifd.set(t::MAKE, Value::Ascii("SONY".into()));
        ifd.set(t::MODEL, Value::Ascii(model.into()));
        ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![bits; 3]));
        ifd.set(t::COMPRESSION, Value::Short(vec![compression]));
        if let Some(child) = sub_ifd {
            ifd.set_child(t::SUB_IFDS, child);
        }
        probe(&write(&[ifd]))
    };
    let raw_block = || Some(rgb_ifd(64, 48)); // stands in for the A100's raw-data pointer
    assert_eq!(sony("DSC-F828", 14, 1, None), Some(RawFormat::Arw), "SRF");
    assert_eq!(sony("DSC-F828", 12, 1, None), Some(RawFormat::Arw), "12-bit samples count too");
    assert_eq!(sony("DSLR-A100", 8, 6, raw_block()), Some(RawFormat::Arw), "A100: old-JPEG IFD0 and SubIFDs");
    assert_eq!(sony("DSC-F828", 8, 1, None), None, "an 8-bit TIFF");
    assert_eq!(sony("DSC-F828", 16, 1, None), None, "a 16-bit TIFF");
    // TIFFs exported from A100 photos, with a thumbnail in SubIFDs (Codex review of storytold/lightcraft#702)
    assert_eq!(sony("DSLR-A100", 8, 1, Some(rgb_ifd(64, 48))), None, "an 8-bit A100 TIFF with a SubIFDs thumbnail");
    assert_eq!(sony("DSLR-A100", 16, 8, Some(rgb_ifd(64, 48))), None, "a 16-bit A100 TIFF with a SubIFDs thumbnail");
    assert_eq!(sony("ILCE-7M4", 8, 6, raw_block()), None, "the A100's layout under another model");
}

// --- containers that are recognised but not decoded, with a preview ---

#[test]
fn private_containers_are_recognised_by_their_magic() {
    assert_eq!(probe(b"II\x1a\0\0\0HEAPCCDR\0\0"), Some(RawFormat::Crw));
    assert_eq!(probe(b"MM\0\0\0\x1aHEAPCCDR\0\0"), Some(RawFormat::Crw), "a big-endian heap is a CRW too");
    assert_eq!(probe(b"\0MRM\0\x01\0\0"), Some(RawFormat::Mrw));
    assert_eq!(probe(b"FOVb\x02\0\x02\0"), Some(RawFormat::X3f));
    assert_eq!(RawFormat::Crw.is_supported(), cfg!(feature = "crw"));
    assert!(RawFormat::Dng.is_supported());
    assert_eq!(RawFormat::Mrw.is_supported(), cfg!(feature = "mrw"));
    assert!(!RawFormat::X3f.is_supported());
    // a CRW signature with no heap behind it is corrupt, not a panic
    #[cfg(feature = "crw")]
    assert!(matches!(decode(b"II\x1a\0\0\0HEAPCCDR\0\0"), Err(RawError::Corrupt(_))));
    assert!(matches!(decode(b"FOVb\x02\0\x02\0"), Err(RawError::Unsupported(w)) if w.contains("X3f")));
    // too short for the CIFF signature
    assert_eq!(probe(b"II\x1a\0\0\0HEAP"), None);
}

/// An IFD0 with a JPEG pointer but no `ImageWidth` / `ImageLength` (the Kodak KDC layout) is a raw
/// container with a preview; a TIFF reader would call it malformed.
#[test]
fn ifd0_without_dimensions_but_with_a_jpeg_is_a_preview_container() {
    let jpeg: Vec<u8> = [&[0xffu8, 0xd8, 0xff, 0xc0, 0, 0x0b, 8, 0, 1, 0, 1, 1, 1, 0x11, 0][..], &[0x55; 40], &[0xff, 0xd9]].concat();
    let off = 8 + 2 + 24 + 4;
    let mut f = b"II*\0\x08\0\0\0\x02\0".to_vec();
    for (tag, v) in [(513u16, off as u32), (514, jpeg.len() as u32)] {
        f.extend_from_slice(&tag.to_le_bytes());
        f.extend_from_slice(&[4, 0, 1, 0, 0, 0]);
        f.extend_from_slice(&v.to_le_bytes());
    }
    f.extend_from_slice(&[0, 0, 0, 0]);
    f.extend_from_slice(&jpeg);
    assert_eq!(probe(&f), Some(RawFormat::OtherTiff));
    assert!(matches!(decode(&f), Err(RawError::Unsupported(_))));
    assert_eq!(embedded_preview(&f), Some(jpeg));
    // no pointer and no sub-IFDs: just a broken TIFF
    let g = write(&[IfdBuilder::new().with(t::MAKE, Value::Ascii("KODAK".into()))]);
    assert_eq!(probe(&g), None);
}

/// A GoPro GPR (a DNG with VC-5 coded raw data, Compression 9) is refused by the full decode with
/// a reason that names the format, whatever its tile bytes hold; the header probe still
/// describes it (so it imports).
#[test]
fn gopro_vc5_dng_is_unsupported_with_a_clear_reason() {
    let mut ifd = IfdBuilder::new();
    ifd.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
    ifd.set(t::IMAGE_WIDTH, Value::Long(vec![32]));
    ifd.set(t::IMAGE_LENGTH, Value::Long(vec![16]));
    ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
    ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
    ifd.set(t::PHOTOMETRIC, Value::Short(vec![32803]));
    ifd.set(t::COMPRESSION, Value::Short(vec![9]));
    ifd.set(t::DNG_VERSION, Value::Byte(vec![1, 4, 0, 0]));
    ifd.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
    ifd.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
    ifd.set_image(ImageData::Tiles { tile_width: 32, tile_height: 16, tiles: vec![vec![0x5au8; 400]] });
    let bytes = write(&[ifd]);
    assert_eq!(probe(&bytes), Some(RawFormat::Dng));
    let want = "DNG compression 9 (GoPro VC-5) is not decoded yet";
    let Err(RawError::Unsupported(why)) = decode(&bytes) else { panic!("expected Unsupported from decode") };
    assert_eq!(why, want);
    let info = probe_info(&bytes).expect("header probe describes the file");
    assert_eq!((info.width, info.height), (32, 16));
}

/// A LinearRaw DNG in the layout of Lightroom's merges: a `w` × `h` main image (samples 1000) with a default
/// crop, and reduced copies (samples 2000 + their width) of the given sizes beside it.
fn linear_dng_with_copies(w: u32, h: u32, copies: &[(u32, u32)], main_tags: &[(u16, Value)]) -> Vec<u8> {
    let linear = |subfile: u32, w: u32, h: u32, sample: u16| {
        let mut ifd = IfdBuilder::new();
        ifd.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![subfile]));
        ifd.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
        ifd.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
        ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![16, 16, 16]));
        ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
        ifd.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::LINEAR_RAW]));
        ifd.set(t::COMPRESSION, Value::Short(vec![1]));
        ifd.set(t::WHITE_LEVEL, Value::Long(vec![32768]));
        let samples: Vec<u8> = std::iter::repeat_n(sample.to_le_bytes(), (w * h * 3) as usize).flatten().collect();
        ifd.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![samples] });
        ifd
    };
    let mut main = linear(0, w, h, 1000);
    main.set(t::BLACK_LEVEL, Value::Short(vec![100]));
    main.set(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![4, 2]));
    main.set(t::DEFAULT_CROP_SIZE, Value::Long(vec![w - 8, h - 4]));
    for (tag, v) in main_tags {
        main.set(*tag, v.clone());
    }
    let mut ifd0 = rgb_ifd(4, 2);
    ifd0.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![1]));
    ifd0.set(t::DNG_VERSION, Value::Byte(vec![1, 7, 0, 0]));
    ifd0.set(t::MAKE, Value::Ascii("DJI".into()));
    ifd0.set(t::COLOR_MATRIX_1, Value::SRational(vec![(1, 1), (0, 1), (0, 1), (0, 1), (1, 1), (0, 1), (0, 1), (0, 1), (1, 1)]));
    ifd0.add_sub_ifd(main);
    for &(cw, ch) in copies {
        ifd0.add_sub_ifd(linear(1, cw, ch, 2000 + cw as u16));
    }
    write(&[ifd0])
}

/// A thumbnail of a Lightroom merge decodes the smallest reduced copy of its raw image that covers it, with the
/// main image's levels and its crop scaled; nothing covers it, or the main image has full-size opcodes: `None`.
#[test]
fn reduced_copies_of_a_linear_raw_dng_decode_at_their_size() {
    let file = linear_dng_with_copies(64, 32, &[(32, 16), (16, 8)], &[]);
    let full = decode(&file).unwrap();
    assert_eq!((full.width, full.height, full.crop), (64, 32, Rect::new(4, 2, 56, 28)));
    // a 12 px picture: the 16 px copy (its crop is 14 px long)
    let small = decode_reduced(&file, 12, &Limits::default()).unwrap().expect("a copy covers 12 px");
    assert_eq!((small.width, small.height), (16, 8));
    assert_eq!(small.crop, Rect::new(1, 1, 14, 7));
    let RawData::U16(v) = &small.data else { panic!("16-bit samples") };
    assert!(v.iter().all(|&s| s == 2016), "the 16 px copy's samples");
    assert_eq!((small.white.clone(), small.black.values.clone()), (vec![32768.0], vec![100.0]), "its own white, the main black");
    // 20 px: the 32 px copy (28 px); 40 px: none covers it
    assert_eq!(decode_reduced(&file, 20, &Limits::default()).unwrap().map(|r| r.width), Some(32));
    assert!(decode_reduced(&file, 40, &Limits::default()).unwrap().is_none());
    // a different aspect ratio is not a copy of the image
    assert!(decode_reduced(&linear_dng_with_copies(64, 32, &[(16, 16)], &[]), 8, &Limits::default()).unwrap().is_none());
    // opcodes are in full-size pixels
    let opcodes = linear_dng_with_copies(64, 32, &[(16, 8)], &[(t::OPCODE_LIST_3, Value::Undefined(vec![0, 0, 0, 0]))]);
    assert!(decode_reduced(&opcodes, 8, &Limits::default()).unwrap().is_none());
    // not a DNG
    assert!(decode_reduced(&write(&[rgb_ifd(16, 8)]), 8, &Limits::default()).unwrap().is_none());
}

/// Compression 99 (not a registered TIFF value; Leaf MOS tiles) marks a raw even without a CFA tag.
#[test]
fn private_compression_99_is_a_raw() {
    let mut ifd = rgb_ifd(16, 12);
    ifd.set(t::COMPRESSION, Value::Short(vec![99]));
    assert_eq!(probe(&write(&[ifd])), Some(RawFormat::OtherTiff));
}

/// storytold/lightcraft#1275: an uncompressed Leaf MOS (Aptus 22) is a full-size 16-bit grey IFD0 with no Make tag; only Leaf's
/// data block (`0x8606`) says it is a raw. It must not import as a greyscale image of its mosaic. The same IFD
/// without the block, or an RGB image carrying it, stays an ordinary image.
#[test]
fn uncompressed_leaf_mos_is_a_raw() {
    let mosaic = |leaf_data: bool| {
        let mut ifd = IfdBuilder::new();
        ifd.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        ifd.set(t::IMAGE_WIDTH, Value::Long(vec![16]));
        ifd.set(t::IMAGE_LENGTH, Value::Long(vec![12]));
        ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
        ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        ifd.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::BLACK_IS_ZERO]));
        ifd.set(t::COMPRESSION, Value::Short(vec![1]));
        ifd.set_image(ImageData::Tiles { tile_width: 16, tile_height: 12, tiles: vec![vec![0x40; 16 * 12 * 2]] });
        if leaf_data {
            ifd.set(LEAF_DATA, Value::Undefined(b"PKTS\0\0\0\x01".to_vec()));
        }
        ifd
    };
    let bytes = TiffWriter::new(craft_tiff::ByteOrder::Big, false).write(&[mosaic(true)]).unwrap();
    assert_eq!(probe(&bytes), Some(RawFormat::OtherTiff));
    assert!(matches!(decode(&bytes), Err(RawError::Unsupported(_))));
    assert!(matches!(probe_info(&bytes), Err(RawError::Unsupported(_))));
    assert_eq!(probe(&write(&[mosaic(false)])), None, "a 16-bit grey TIFF");
    let mut rgb = rgb_ifd(16, 12);
    rgb.set(LEAF_DATA, Value::Undefined(b"PKTS\0\0\0\x01".to_vec()));
    assert_eq!(probe(&write(&[rgb])), None, "a rendered RGB image that kept the block");
}

/// storytold/lightcraft#1275: a Hasselblad Lunar is a Sony NEX-7 ARW under Make HASSELBLAD. Sony's raw coding (compression 32767
/// or the tone-curve tag `0x7010`) sends it to the ARW reader; a Hasselblad file without it does not go there.
#[test]
fn hasselblad_make_with_sony_raw_coding_is_arw() {
    let file = |make: &str, compression: u16, tone_curve: bool| {
        let mut raw = IfdBuilder::new();
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![64]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![2]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![12]));
        raw.set(t::COMPRESSION, Value::Short(vec![compression]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        if tone_curve {
            raw.set(0x7010, Value::Short(vec![8000, 10400, 12900, 14100]));
        }
        raw.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![vec![0; 128]] });
        let mut ifd0 = rgb_ifd(4, 3);
        ifd0.set(t::MAKE, Value::Ascii(make.into()));
        ifd0.set(t::MODEL, Value::Ascii("Lunar".into()));
        ifd0.add_sub_ifd(raw);
        write(&[ifd0])
    };
    assert_eq!(probe(&file("HASSELBLAD", 32767, false)), Some(RawFormat::Arw));
    assert_eq!(probe(&file("Hasselblad", 1, true)), Some(RawFormat::Arw));
    // a Hasselblad CFA image in another coding is not an ARW
    assert_ne!(probe(&file("HASSELBLAD", 1, false)), Some(RawFormat::Arw));
    assert_ne!(probe(&file("HASSELBLAD", 7, false)), Some(RawFormat::Arw));
}

#[cfg(feature = "hasselblad")]
mod cfa_tiff {
    use super::*;

    // --- TIFF raws that describe their raw IFD the way DNG does but carry no DNG version tag ---

    /// 8 x 6 samples, 16 bits, value `1000 + 100 y + x`.
    fn cfa_samples() -> Vec<u16> {
        (0..48u16).map(|i| 1000 + 100 * (i / 8) + i % 8).collect()
    }

    /// A small raw in the layout of the Hasselblad 3FR/FFF family: IFD0 is a reduced RGB image with the
    /// colour tags, a SubIFD holds the full-resolution CFA image (no `CFAPattern`, DNG-style levels and
    /// crop). `strip` is the CFA image's data and `compression` its coding.
    fn plain_cfa_tiff(compression: u16, strip: Vec<u8>, pattern: Option<[u8; 4]>) -> Vec<u8> {
        let mut raw = IfdBuilder::new();
        raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![8]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![6]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
        raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        raw.set(t::COMPRESSION, Value::Short(vec![compression]));
        raw.set(t::BLACK_LEVEL, Value::Rational(vec![(256, 1)]));
        raw.set(t::WHITE_LEVEL, Value::Long(vec![60000]));
        raw.set(t::DEFAULT_CROP_ORIGIN, Value::Short(vec![2, 2]));
        raw.set(t::DEFAULT_CROP_SIZE, Value::Short(vec![4, 2]));
        if let Some(p) = pattern {
            raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
            raw.set(t::CFA_PATTERN_EP, Value::Byte(p.to_vec()));
        }
        raw.set_image(ImageData::Strips { rows_per_strip: 6, strips: vec![strip] });
        let mut ifd0 = rgb_ifd(4, 3);
        ifd0.set(t::MAKE, Value::Ascii("Hasselblad".into()));
        ifd0.set(t::COLOR_MATRIX_1, Value::SRational(vec![(5, 10), (-1, 10), (0, 10), (-5, 10), (12, 10), (3, 10), (-1, 10), (2, 10), (6, 10)]));
        ifd0.set(t::AS_SHOT_NEUTRAL, Value::Rational(vec![(2, 5), (1, 1), (3, 5)]));
        ifd0.add_sub_ifd(raw);
        write(&[ifd0])
    }

    fn le_words(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    /// Known answer: samples, black and white level, crop and colour come from the file's own tags; the
    /// layout it does not state is RGGB.
    #[test]
    fn uncompressed_cfa_tiff_without_a_dng_version_decodes() {
        let bytes = plain_cfa_tiff(1, le_words(&cfa_samples()), None);
        assert_eq!(probe(&bytes), Some(RawFormat::CfaTiff));
        let r = decode(&bytes).unwrap();
        assert_eq!((r.format, r.width, r.height, r.bits), (RawFormat::CfaTiff, 8, 6, 16));
        assert_eq!(r.data, RawData::U16(cfa_samples()));
        assert_eq!(r.cfa.as_ref().unwrap().name(), "RGGB");
        assert_eq!((r.black.mean(), r.white.clone()), (256.0, vec![60000.0]));
        assert_eq!(r.crop, Rect::new(2, 2, 4, 2));
        assert!(r.color.color_matrix[0].is_some());
        assert_eq!(r.color.as_shot_neutral, Some([0.4, 1.0, 0.6]));
        // headers only: the same description without samples
        let i = probe_info(&bytes).unwrap();
        assert_eq!((i.format, i.cfa.as_ref().map(Cfa::name), i.crop), (RawFormat::CfaTiff, Some("RGGB".to_string()), Rect::new(2, 2, 4, 2)));
    }

    /// Without DNG's white-balance tag the file is left alone (a Kodak or Sinar TIFF with a CFA IFD, say).
    #[test]
    fn cfa_tiff_without_a_white_balance_tag_is_not_claimed() {
        let mut raw = IfdBuilder::new();
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![8]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![6]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
        raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        raw.set(t::COMPRESSION, Value::Short(vec![1]));
        raw.set_image(ImageData::Strips { rows_per_strip: 6, strips: vec![le_words(&cfa_samples())] });
        let mut ifd0 = rgb_ifd(4, 3);
        ifd0.add_sub_ifd(raw);
        assert_eq!(probe(&write(&[ifd0])), Some(RawFormat::OtherTiff));
    }

    /// A stated `CFAPattern` wins over the RGGB default.
    #[test]
    fn cfa_tiff_keeps_a_stated_pattern() {
        let bytes = plain_cfa_tiff(1, le_words(&cfa_samples()), Some([2, 1, 1, 0]));
        assert_eq!(decode(&bytes).unwrap().cfa.unwrap().name(), "BGGR");
    }

    /// The lossless-JPEG coding of the same layout goes through the lossless decoder (one strip for the
    /// whole image, as the 3FR/FFF files have it).
    #[test]
    fn lossless_jpeg_cfa_tiff_decodes() {
        let strip = ljpeg::encode(&cfa_samples(), 8, 6, 1, 16, 1, 0);
        let bytes = plain_cfa_tiff(7, strip, None);
        assert_eq!(probe(&bytes), Some(RawFormat::CfaTiff));
        let r = decode(&bytes).unwrap();
        assert_eq!(r.data, RawData::U16(cfa_samples()));
        assert_eq!(r.cfa.unwrap().name(), "RGGB");
    }

    /// Hasselblad's own lossless-JPEG coding (selection value 8, as in FFF and most compressed 3FR files) of the
    /// same layout decodes: the file is read like the uncompressed one.
    #[test]
    fn selection_value_8_cfa_tiff_decodes() {
        let strip = ljpeg::tests::encode_hasselblad(&cfa_samples(), 8, 6);
        let bytes = plain_cfa_tiff(7, strip, None);
        assert_eq!(probe(&bytes), Some(RawFormat::CfaTiff));
        let r = decode(&bytes).unwrap();
        assert_eq!((r.format, r.width, r.height), (RawFormat::CfaTiff, 8, 6));
        assert_eq!(r.data, RawData::U16(cfa_samples()));
        assert_eq!(r.cfa.unwrap().name(), "RGGB");
        assert_eq!(probe_info(&bytes).unwrap().format, RawFormat::CfaTiff);
    }

    /// A lossless-JPEG scan whose predictor selection value is neither 0 to 7 (T.81) nor 8 (Hasselblad) is not a
    /// coding the lossless decoder knows: the file stays an undecodable raw, with that as the reason, and keeps the
    /// stand-in preview an unsupported container gets.
    #[test]
    fn cfa_tiff_with_an_unknown_predictor_stays_undecodable() {
        let mut strip = ljpeg::encode(&cfa_samples(), 8, 6, 1, 16, 1, 0);
        let sos = strip.windows(2).position(|w| w == [0xff, 0xda]).unwrap();
        let ss = sos + 2 + 2 + 1 + 2;
        assert_eq!(strip[ss], 1, "selection value byte");
        strip[ss] = 9;
        assert!(matches!(ljpeg::decode(&strip, 1 << 20), Err(RawError::Unsupported(w)) if w.contains("selection value 9")));
        let bytes = plain_cfa_tiff(7, strip, None);
        assert_eq!(probe(&bytes), Some(RawFormat::OtherTiff));
        assert!(!RawFormat::OtherTiff.is_supported());
        assert_eq!(embedded_preview(&bytes), None, "no JPEG in the file, and an undecodable raw gets no RGB stand-in");
        let Err(RawError::Unsupported(why)) = decode(&bytes) else { panic!("expected Unsupported") };
        assert!(why.contains("lossless JPEG") && why.contains("selection value 9"), "{why}");
    }

    /// Other codings of a CFA IFD (here Deflate) are not claimed by the plain reader.
    #[test]
    fn cfa_tiff_with_another_coding_is_not_claimed() {
        let bytes = plain_cfa_tiff(8, vec![0; 96], None);
        assert_eq!(probe(&bytes), Some(RawFormat::OtherTiff));
    }

    /// PhotoCraft's reader: four black levels without a `BlackLevelRepeatDim` are one per site of the 2 × 2 CFA in a
    /// file that is not a DNG. The DNG reader took the first for every site. In a DNG the count must match the repeat.
    #[test]
    fn four_black_levels_without_a_repeat_are_per_cfa_site_outside_dng() {
        let with_blacks = |dng: bool| {
            let mut raw = IfdBuilder::new();
            raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
            raw.set(t::IMAGE_WIDTH, Value::Long(vec![8]));
            raw.set(t::IMAGE_LENGTH, Value::Long(vec![6]));
            raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
            raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
            raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
            raw.set(t::COMPRESSION, Value::Short(vec![1]));
            raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
            raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
            raw.set(t::BLACK_LEVEL, Value::Short(vec![250, 252, 254, 256]));
            raw.set_image(ImageData::Strips { rows_per_strip: 6, strips: vec![le_words(&cfa_samples())] });
            let mut ifd0 = rgb_ifd(4, 3);
            ifd0.set(t::MAKE, Value::Ascii("Hasselblad".into()));
            ifd0.set(t::AS_SHOT_NEUTRAL, Value::Rational(vec![(2, 5), (1, 1), (3, 5)]));
            if dng {
                ifd0.set(t::DNG_VERSION, Value::Byte(vec![1, 4, 0, 0]));
            }
            ifd0.add_sub_ifd(raw);
            decode(&write(&[ifd0])).unwrap().black
        };
        let b = with_blacks(false);
        assert_eq!((b.repeat_rows, b.repeat_cols, b.values.clone()), (2, 2, vec![250.0, 252.0, 254.0, 256.0]));
        assert_eq!((b.at(0, 0, 0, 1), b.at(1, 0, 0, 1), b.at(0, 1, 0, 1), b.at(3, 3, 0, 1)), (250.0, 252.0, 254.0, 256.0));
        assert_eq!(with_blacks(true), BlackLevel::uniform(250.0), "a DNG's count has to match its repeat");
    }
}

/// A Samsung-branded body whose maker note has a Pentax layout is read by the Pentax reader; another
/// Samsung file stays with the Samsung format.
#[test]
fn samsung_make_with_a_pentax_maker_note_is_pef() {
    let with_note = |note: &[u8], cfa: bool| {
        let mut ifd = rgb_ifd(16, 12);
        if cfa {
            ifd.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        }
        ifd.set(t::MAKE, Value::Ascii("SAMSUNG TECHWIN".into()));
        let mut exif = IfdBuilder::new();
        exif.set(t::MAKER_NOTE, Value::Undefined(note.to_vec()));
        ifd.set_child(t::EXIF_IFD, exif);
        write(&[ifd])
    };
    assert_eq!(probe(&with_note(b"AOC\0MM\0\x01\0\0\0\0\0\0", true)), Some(RawFormat::Pef));
    assert_eq!(probe(&with_note(b"PENTAX \0MM\0\0\0\0\0\0\0", true)), Some(RawFormat::Pef));
    assert_eq!(probe(&with_note(b"STMN100\0\0\0\0\0\0\0\0\0", true)), Some(RawFormat::Srw));
    // without raw evidence (an RGB image from such a body) neither reader claims it (storytold/lightcraft#281)
    assert_eq!(probe(&with_note(b"AOC\0MM\0\x01\0\0\0\0\0\0", false)), None);
    assert_eq!(probe(&with_note(b"STMN100\0\0\0\0\0\0\0\0\0", false)), None);
}

#[test]
fn probe_rejects_junk() {
    assert_eq!(probe(b""), None);
    assert_eq!(probe(b"hello world, not a raw"), None);
    assert_eq!(probe(b"\0\0\0\x18ftypcrx \0\0\0\x01"), Some(RawFormat::Cr3));
    assert_eq!(probe(b"FUJIFILMCCD-RAW 0201"), Some(RawFormat::Raf));
    assert!(decode(b"FUJIFILMCCD-RAW 0201").is_err());
    assert_eq!(decode(b"junk"), Err(RawError::NotRaw));
}
