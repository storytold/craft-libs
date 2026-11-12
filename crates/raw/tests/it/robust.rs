//! Malformed-input robustness: decoding random or corrupted files must never panic.

use craft_raw::{BlackLevel, Cfa, ColorData, DngCompression, DngWriteOptions, Limits, OpcodeLists, Orientation, RawData, RawFormat, RawImage, Rect};
use proptest::prelude::*;

fn sample(comp: DngCompression) -> Vec<u8> {
    let (w, h) = (24usize, 16usize);
    let data: Vec<u16> = (0..w * h).map(|i| (i * 97 % 4000) as u16).collect();
    let raw = RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16(data),
        cfa: Some(Cfa::bayer("RGGB").unwrap()),
        bits: 12,
        black: BlackLevel::uniform(64.0),
        white: vec![4095.0],
        active_area: Rect::new(0, 0, w, h),
        crop: Rect::new(0, 0, w, h),
        orientation: Orientation::Normal,
        color: ColorData::default(),
        wb_multipliers: None,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: Default::default(),
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    };
    craft_raw::write_dng(&raw, &DngWriteOptions { compression: comp, ..Default::default() }).unwrap()
}

/// Everything the crate does with a file. A decoded image is internally consistent, and the header probe
/// succeeds whenever the decode does.
fn exercise(bytes: &[u8]) {
    let _ = craft_raw::probe(bytes);
    let _ = craft_raw::embedded_preview(bytes);
    let _ = craft_raw::embedded_preview_covering(bytes, 256);
    let _ = craft_raw::embedded_preview_color_space(bytes);
    let _ = craft_raw::semantic_masks(bytes);
    let _ = craft_raw::decode_reduced(bytes, 64, &Limits::default());
    let info = craft_raw::probe_info(bytes);
    if let Ok(img) = craft_raw::decode(bytes) {
        img.validate().expect("a decoded image validates");
        assert_eq!(info.as_ref().ok(), Some(&img.info()), "the header probe describes the decoded image");
    }
}

const COMPRESSIONS: [DngCompression; 3] =
    [DngCompression::Uncompressed, DngCompression::Lj92 { tile: 16 }, DngCompression::Deflate { tile: 16, half: false }];

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, .. ProptestConfig::default() })]

    #[test]
    fn mutated_dngs_never_panic(flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..24), cut in any::<usize>(), comp in 0usize..3) {
        let mut data = sample(COMPRESSIONS[comp]);
        for (i, v) in flips {
            let n = data.len();
            data[i % n] = v;
        }
        let keep = if cut % 3 == 0 { cut % data.len() } else { data.len() };
        exercise(&data[..keep]);
    }

    #[test]
    fn random_tiff_like_bytes_never_panic(mut data in proptest::collection::vec(any::<u8>(), 8..700), kind in 0usize..4) {
        let heads: [&[u8]; 4] = [b"II*\0\x08\0\0\0", b"MM\0*\0\0\0\x08", b"II*\0\x10\0\0\0CR\x02\0", b"II+\0\x08\0\0\0"];
        let h = heads[kind];
        let n = h.len().min(data.len());
        data[..n].copy_from_slice(&h[..n]);
        exercise(&data);
    }

    #[test]
    fn random_lj92_never_panics(data in proptest::collection::vec(any::<u8>(), 0..400)) {
        let mut s = vec![0xff, 0xd8, 0xff, 0xc3, 0, 11, 12, 0, 8, 0, 8, 1, 1, 0x11, 0];
        s.extend_from_slice(&data);
        let _ = craft_raw::ljpeg::decode(&s, 1 << 20);
        let _ = craft_raw::ljpeg::decode_hasselblad(&s, 1 << 20);
        let _ = craft_raw::ljpeg::decode_subsampled(&s, 1 << 20, craft_raw::ljpeg::Prediction::Sequential);
    }

    #[test]
    fn random_opcode_lists_and_gain_maps_never_panic(data in proptest::collection::vec(any::<u8>(), 0..300), count in 0u32..6, id in 0u32..16) {
        let mut list = count.to_be_bytes().to_vec();
        list.extend_from_slice(&id.to_be_bytes());
        list.extend_from_slice(&data);
        let parsed = craft_raw::opcodes::parse_list(&list);
        prop_assert_eq!(craft_raw::opcodes::parse_list(&craft_raw::opcodes::write_list(&parsed)), parsed);
        for order in [craft_tiff::ByteOrder::Little, craft_tiff::ByteOrder::Big] {
            for v2 in [false, true] {
                let _ = craft_raw::gaintable::GainTableMap::parse(&data, order, v2);
            }
        }
    }

    #[cfg(feature = "crw")]
    #[test]
    fn random_ciff_like_bytes_never_panic(data in proptest::collection::vec(any::<u8>(), 0..600), big in any::<bool>()) {
        let mut f = if big { b"MM".to_vec() } else { b"II".to_vec() };
        f.extend_from_slice(&[26, 0, 0, 0]);
        f.extend_from_slice(b"HEAPCCDR");
        f.extend_from_slice(&data);
        prop_assert_eq!(craft_raw::probe(&f), Some(RawFormat::Crw));
        exercise(&f);
    }
}

#[test]
fn truncation_at_every_length() {
    for comp in COMPRESSIONS {
        let d = sample(comp);
        assert!(craft_raw::decode(&d).is_ok());
        for n in 0..d.len() {
            exercise(&d[..n]);
        }
    }
}

#[test]
fn every_single_byte_mutation_of_the_header_is_safe() {
    // the IFD and its values sit at the end of the file the writer makes: flip every byte of the last 600
    let d = sample(DngCompression::Uncompressed);
    for i in d.len().saturating_sub(600)..d.len() {
        for v in [0x00, 0xff, d[i] ^ 0x80, d[i].wrapping_add(1)] {
            let mut m = d.clone();
            m[i] = v;
            exercise(&m);
        }
    }
    for i in 0..8 {
        let mut m = d.clone();
        m[i] ^= 0xff;
        exercise(&m);
    }
}

#[test]
fn hostile_dimensions_are_rejected_quickly() {
    // a DNG claiming a 60000 × 60000 16-bit image with 10 bytes of data
    use craft_tiff::tags as t;
    use craft_tiff::{IfdBuilder, ImageData, TiffWriter, Value};
    let mut ifd = IfdBuilder::new();
    ifd.set(t::DNG_VERSION, Value::Byte(vec![1, 4, 0, 0]));
    ifd.set(t::IMAGE_WIDTH, Value::Long(vec![60000]));
    ifd.set(t::IMAGE_LENGTH, Value::Long(vec![60000]));
    ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
    ifd.set(t::PHOTOMETRIC, Value::Short(vec![32803]));
    ifd.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
    ifd.set(t::COMPRESSION, Value::Short(vec![7]));
    ifd.set_image(ImageData::Tiles { tile_width: 60000, tile_height: 60000, tiles: vec![vec![0xff, 0xd8, 0xff, 0xd9]] });
    let bytes = TiffWriter::default().write(&[ifd]).unwrap();
    let t0 = std::time::Instant::now();
    assert!(craft_raw::decode(&bytes).is_err());
    assert!(t0.elapsed().as_secs_f64() < 2.0);
}
