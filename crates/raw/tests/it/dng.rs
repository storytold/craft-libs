//! Synthetic DNG round trips: every storage variant we decode is written with our own TIFF writer (and LJ92
//! encoder) and must decode bit-exactly.

use craft_raw::opcodes::{Area, Opcode};
use craft_raw::{
    BlackLevel, Cfa, ColorData, DngCompression, DngWriteOptions, OpcodeLists, Orientation, RawData, RawFormat, RawImage, Rect, decode,
    embedded_preview, probe, write_dng,
};
use craft_tiff::tags::{self as t, compression, photometric};
use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};

fn scene_value(x: usize, y: usize, c: u8, max: f32) -> f32 {
    let base = [0.55, 0.8, 0.4][c as usize];
    let v = base * (0.2 + 0.7 * ((x as f32 * 0.13).sin() * 0.5 + 0.5) * ((y as f32 * 0.07).cos() * 0.3 + 0.7));
    v * max
}

fn synthetic(w: usize, h: usize, cfa: Option<Cfa>, cpp: usize) -> RawImage {
    let black = 256.0;
    let white = 16000.0;
    let mut data = Vec::with_capacity(w * h * cpp);
    for y in 0..h {
        for x in 0..w {
            for s in 0..cpp {
                let c = match &cfa {
                    Some(p) => p.color_at(x, y),
                    None => s as u8,
                };
                data.push((black + scene_value(x, y, c, white - black)).round() as u16);
            }
        }
    }
    let color = ColorData {
        illuminant: [17, 21],
        color_matrix: [
            Some([[0.9, 0.2, -0.15], [-0.3, 1.25, 0.08], [0.02, -0.12, 0.85]]),
            Some([[0.7, 0.3, -0.1], [-0.35, 1.3, 0.1], [0.05, -0.2, 1.0]]),
        ],
        as_shot_neutral: Some([0.5, 1.0, 0.7]),
        baseline_exposure: 0.35,
        baseline_sharpness: Some(1.5),
        ..Default::default()
    };
    let camera = craft_raw::Camera { make: Some("Synth".into()), model: Some("Cam 1".into()), ..Default::default() };
    RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp,
        data: RawData::U16(data),
        cfa,
        bits: 14,
        black: BlackLevel::uniform(black),
        white: vec![white],
        active_area: Rect::new(2, 1, w - 4, h - 2),
        crop: Rect::new(3, 2, w - 12, h - 8),
        orientation: Orientation::Rotate90,
        color,
        wb_multipliers: None,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera,
        cinema: Default::default(),
        lens_metadata: Default::default(),
        level_sources: Default::default(),
    }
}

fn assert_same(a: &RawImage, b: &RawImage) {
    assert_eq!((a.width, a.height, a.cpp), (b.width, b.height, b.cpp));
    assert_eq!(a.data, b.data);
    assert_eq!(a.cfa, b.cfa);
    assert_eq!(a.active_area, b.active_area);
    assert_eq!(a.crop, b.crop);
    assert_eq!(a.orientation, b.orientation);
    assert!((0..a.cpp).all(|s| a.white_at(s) == b.white_at(s)));
    assert_eq!(a.black.values, b.black.values.iter().take(a.black.values.len()).copied().collect::<Vec<_>>());
    for i in 0..2 {
        let (x, y) = (a.color.color_matrix[i].unwrap(), b.color.color_matrix[i].unwrap());
        for r in 0..3 {
            for c in 0..3 {
                assert!((x[r][c] - y[r][c]).abs() < 1e-6);
            }
        }
    }
    assert_eq!(a.color.illuminant, b.color.illuminant);
    let (n1, n2) = (a.color.as_shot_neutral.unwrap(), b.color.as_shot_neutral.unwrap());
    assert!((0..3).all(|i| (n1[i] - n2[i]).abs() < 1e-6));
    assert!((a.color.baseline_exposure - b.color.baseline_exposure).abs() < 1e-6);
    assert!(b.color.baseline_sharpness.is_some_and(|v| (v - 1.5).abs() < 1e-6), "{:?}", b.color.baseline_sharpness);
    assert_eq!(b.camera.make.as_deref(), Some("Synth"));
    assert_eq!(b.camera.model.as_deref(), Some("Cam 1"));
}

#[test]
fn writer_camera_identity_tags_roundtrip() {
    let cases = [
        (Some("Canon"), Some("Canon EOS R6"), "Canon EOS R6"),
        (Some("CANON"), Some("cAnOn EOS R6"), "cAnOn EOS R6"),
        (Some("Canon"), Some("Canon"), "Canon"),
        (Some("Canon"), Some("Canon-EOS R6"), "Canon-EOS R6"),
        (Some("Canon"), Some("Canon_EOS R6"), "Canon Canon_EOS R6"),
        (Some("Canon"), Some("Canonic R6"), "Canon Canonic R6"),
        (Some("Canon"), Some("Canon2 R6"), "Canon Canon2 R6"),
        (Some("Canon"), Some("Canoné R6"), "Canon Canoné R6"),
        (Some("Canon"), Some("EOS R6"), "Canon EOS R6"),
        (Some("Eastman Kodak"), Some("eAsTmAn kOdAk DCS Pro"), "eAsTmAn kOdAk DCS Pro"),
        (Some("Eastman Kodak"), Some("Eastman Kodakish DCS Pro"), "Eastman Kodak Eastman Kodakish DCS Pro"),
        (Some("Eastman Kodak"), Some("Eastman DCS Pro"), "Eastman Kodak Eastman DCS Pro"),
        (Some(""), Some("EOS R6"), "EOS R6"),
        (Some("Canon"), Some(""), "Canon"),
        (Some(""), Some(""), "Unknown"),
        (None, Some("EOS R6"), "EOS R6"),
        (Some("Canon"), None, "Canon"),
        (None, None, "Unknown"),
    ];
    for (make, model, unique) in cases {
        let mut raw = synthetic(16, 16, Some(Cfa::bayer("RGGB").unwrap()), 1);
        raw.camera.make = make.map(str::to_owned);
        raw.camera.model = model.map(str::to_owned);
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let bytes =
                write_dng(&raw, &DngWriteOptions { compression: DngCompression::Uncompressed, order, xmp: None, ..Default::default() }).unwrap();
            let tiff = craft_tiff::Tiff::parse(&bytes).unwrap();
            let ifd = &tiff.ifds[0];
            assert_eq!(ifd.value(t::UNIQUE_CAMERA_MODEL), Some(&Value::Ascii(unique.into())), "{make:?} / {model:?}, {order:?}");
            for (tag, original) in [(t::MAKE, make), (t::MODEL, model)] {
                let expected = original.filter(|s| !s.is_empty()).map(|s| Value::Ascii(s.into()));
                assert_eq!(ifd.value(tag), expected.as_ref(), "tag {tag}, {make:?} / {model:?}, {order:?}");
            }
        }
    }
}

#[test]
fn writer_roundtrip_all_layouts() {
    for (w, h) in [(64, 48), (70, 45), (33, 17)] {
        for cfa in [Some(Cfa::bayer("RGGB").unwrap()), Some(Cfa::bayer("GBRG").unwrap()), Some(Cfa::xtrans()), None] {
            let cpp = if cfa.is_some() { 1 } else { 3 };
            let raw = synthetic(w, h, cfa, cpp);
            for comp in [DngCompression::Uncompressed, DngCompression::Lj92 { tile: 16 }, DngCompression::Lj92 { tile: 256 }] {
                for order in [ByteOrder::Little, ByteOrder::Big] {
                    let bytes = write_dng(&raw, &DngWriteOptions { compression: comp, order, xmp: None, ..Default::default() }).unwrap();
                    assert_eq!(probe(&bytes), Some(RawFormat::Dng));
                    let back = decode(&bytes).unwrap();
                    assert_same(&raw, &back);
                    // the header-only probe describes the same image
                    assert_eq!(craft_raw::probe_info(&bytes).unwrap(), back.info());
                }
            }
        }
    }
}

#[test]
fn deflate_roundtrips_integer_and_float() {
    for (w, h) in [(64, 48), (70, 45)] {
        for cfa in [Some(Cfa::bayer("RGGB").unwrap()), None] {
            let cpp = if cfa.is_some() { 1 } else { 3 };
            let raw = synthetic(w, h, cfa, cpp);
            for order in [ByteOrder::Little, ByteOrder::Big] {
                let bytes = write_dng(
                    &raw,
                    &DngWriteOptions { compression: DngCompression::Deflate { tile: 32, half: false }, order, xmp: None, ..Default::default() },
                )
                .unwrap();
                assert_same(&raw, &decode(&bytes).unwrap());
            }
        }
    }
    // linear float data: 32-bit exact, 16-bit within half precision
    let mut raw = synthetic(70, 45, None, 3);
    let vals: Vec<f32> = (0..70 * 45 * 3).map(|i| ((i % 97) as f32 / 97.0).powi(3) * 0.9 + 1e-4).collect();
    raw.data = RawData::F32(vals.clone());
    raw.black = BlackLevel::uniform(0.0);
    raw.white = vec![1.0];
    for half in [false, true] {
        let bytes = write_dng(&raw, &DngWriteOptions { compression: DngCompression::Deflate { tile: 32, half }, ..Default::default() }).unwrap();
        let back = decode(&bytes).unwrap();
        let RawData::F32(b) = &back.data else { panic!("float data expected") };
        for (a, b) in vals.iter().zip(b) {
            let tol = if half { a * 1e-3 + 1e-7 } else { 0.0 };
            assert!((a - b).abs() <= tol, "{a} vs {b} (half: {half})");
        }
    }
    assert_eq!(craft_raw::dngwrite::f32_to_f16(1.0), 0x3c00);
    assert_eq!(craft_raw::dngwrite::f32_to_f16(-2.0), 0xc000);
    assert_eq!(craft_raw::dngwrite::f32_to_f16(1e9), 0x7c00);
    assert_eq!(craft_raw::dngwrite::f32_to_f16(2f32.powi(-24)), 0x0001);
}

// ---------------------------------------------------------------- hand-built variants

fn base_ifd(w: usize, h: usize, bits: u16, cpp: usize, cfa: bool) -> IfdBuilder {
    let mut ifd = IfdBuilder::new();
    ifd.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
    ifd.set(t::IMAGE_WIDTH, Value::Long(vec![w as u32]));
    ifd.set(t::IMAGE_LENGTH, Value::Long(vec![h as u32]));
    ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![bits; cpp]));
    ifd.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![cpp as u16]));
    ifd.set(t::PHOTOMETRIC, Value::Short(vec![if cfa { photometric::CFA } else { photometric::LINEAR_RAW }]));
    if cfa {
        ifd.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
        ifd.set(t::CFA_PATTERN_EP, Value::Byte(vec![1, 0, 2, 1]));
    }
    ifd
}

fn dng(raw: IfdBuilder, order: ByteOrder) -> Vec<u8> {
    dng_with(raw, order, |_| {})
}

/// [`dng`] with extra IFD 0 tags (where DNG writers put camera-profile tags).
fn dng_with(raw: IfdBuilder, order: ByteOrder, extra: impl FnOnce(&mut IfdBuilder)) -> Vec<u8> {
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![1]));
    ifd0.set(t::DNG_VERSION, Value::Byte(vec![1, 6, 0, 0]));
    ifd0.set(t::MAKE, Value::Ascii("Hand".into()));
    ifd0.set(t::COLOR_MATRIX_1, Value::SRational(vec![(1, 1), (0, 1), (0, 1), (0, 1), (1, 1), (0, 1), (0, 1), (0, 1), (1, 1)]));
    ifd0.set(t::CALIBRATION_ILLUMINANT_1, Value::Short(vec![21]));
    ifd0.set(t::AS_SHOT_WHITE_XY, Value::Rational(vec![(3127, 10000), (3290, 10000)]));
    // an 8-bit RGB thumbnail in IFD0 (must not be mistaken for the raw)
    ifd0.set(t::IMAGE_WIDTH, Value::Long(vec![4]));
    ifd0.set(t::IMAGE_LENGTH, Value::Long(vec![2]));
    ifd0.set(t::BITS_PER_SAMPLE, Value::Short(vec![8, 8, 8]));
    ifd0.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
    ifd0.set(t::PHOTOMETRIC, Value::Short(vec![photometric::RGB]));
    ifd0.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![vec![128; 24]] });
    let mut exif = IfdBuilder::new();
    exif.set(t::ISO_SPEED, Value::Short(vec![200]));
    ifd0.set_child(t::EXIF_IFD, exif);
    ifd0.add_sub_ifd(raw);
    extra(&mut ifd0);
    TiffWriter::new(order, false).write(&[ifd0]).unwrap()
}

fn pattern(w: usize, h: usize, cpp: usize, bits: u32) -> Vec<u16> {
    let max = (1u32 << bits) - 1;
    (0..w * h * cpp).map(|i| ((i as u32).wrapping_mul(2654435761u32) >> 7) % (max + 1)).map(|v| v as u16).collect()
}

fn pack_msb(v: &[u16], bits: u32, w: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for row in v.chunks(w) {
        let mut acc = 0u64;
        let mut n = 0;
        for &s in row {
            acc = (acc << bits) | s as u64;
            n += bits;
            while n >= 8 {
                out.push((acc >> (n - 8)) as u8);
                n -= 8;
            }
        }
        if n > 0 {
            out.push((acc << (8 - n)) as u8);
        }
    }
    out
}

fn u16_bytes(v: &[u16], order: ByteOrder) -> Vec<u8> {
    let mut b = Vec::new();
    v.iter().for_each(|&x| order.put_u16(&mut b, x));
    b
}

#[test]
fn packed_bit_depths() {
    let (w, h) = (13, 9);
    for bits in [10u32, 12, 14] {
        let px = pattern(w, h, 1, bits);
        let mut raw = base_ifd(w, h, bits as u16, 1, true);
        raw.set(t::COMPRESSION, Value::Short(vec![1]));
        let strips: Vec<Vec<u8>> = px.chunks(w * 4).map(|c| pack_msb(c, bits, w)).collect();
        raw.set_image(ImageData::Strips { rows_per_strip: 4, strips });
        let img = decode(&dng(raw, ByteOrder::Big)).unwrap();
        assert_eq!(img.data, RawData::U16(px), "{bits} bits");
        assert_eq!(img.white, vec![((1u32 << bits) - 1) as f32]);
        assert_eq!(img.cfa.unwrap().name(), "GRBG");
        assert_eq!(img.camera.make.as_deref(), Some("Hand"));
        assert_eq!(img.color.as_shot_white_xy, Some([0.3127, 0.329]));
    }
}

#[test]
fn deflate_with_integer_predictors() {
    let (w, h, cpp) = (10, 6, 3);
    let px = pattern(w, h, cpp, 16);
    for (pred, factor) in [(1u16, 0usize), (2, 1), (34892, 2), (34893, 4)] {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let mut raw = base_ifd(w, h, 16, cpp, false);
            raw.set(t::COMPRESSION, Value::Short(vec![compression::ADOBE_DEFLATE]));
            raw.set(t::PREDICTOR, Value::Short(vec![pred]));
            raw.set_image(ImageData::Tiles {
                tile_width: 16,
                tile_height: 16,
                tiles: vec![{
                    // one 16×16 tile: re-lay the 10×6 image into the tile grid
                    let mut tile = vec![0u16; 16 * 16 * cpp];
                    for y in 0..h {
                        for x in 0..w * cpp {
                            tile[y * 16 * cpp + x] = px[y * w * cpp + x];
                        }
                    }
                    let mut te = tile.clone();
                    if factor > 0 {
                        for row in te.chunks_mut(16 * cpp) {
                            for i in (cpp * factor..row.len()).rev() {
                                row[i] = row[i].wrapping_sub(row[i - cpp * factor]);
                            }
                        }
                    }
                    miniz_oxide::deflate::compress_to_vec_zlib(&u16_bytes(&te, order), 6)
                }],
            });
            let img = decode(&dng(raw, order)).unwrap();
            assert_eq!(img.data, RawData::U16(px.clone()), "predictor {pred} {order:?}");
            assert!(img.cfa.is_none());
            assert_eq!(img.cpp, 3);
        }
    }
}

fn fp_encode(vals: &[f32], bytes_per: usize, stride: usize, w_samples: usize) -> Vec<u8> {
    // big-endian byte planes per row, then byte differencing
    let mut out = Vec::new();
    for row in vals.chunks(w_samples) {
        let be: Vec<[u8; 4]> = row
            .iter()
            .map(|&v| match bytes_per {
                4 => v.to_be_bytes(),
                2 => {
                    let h = f32_to_f16(v).to_be_bytes();
                    [h[0], h[1], 0, 0]
                }
                _ => {
                    let b = f32_to_f24(v).to_be_bytes();
                    [b[1], b[2], b[3], 0]
                }
            })
            .collect();
        let mut planes = vec![0u8; row.len() * bytes_per];
        for (i, b) in be.iter().enumerate() {
            for k in 0..bytes_per {
                planes[k * row.len() + i] = b[k];
            }
        }
        for i in (stride..planes.len()).rev() {
            planes[i] = planes[i].wrapping_sub(planes[i - stride]);
        }
        out.extend(planes);
    }
    out
}

fn f32_to_f16(v: f32) -> u16 {
    // exact for the test values (multiples of 1/64 in [0, 4))
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    if v == 0.0 {
        return sign;
    }
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let man = ((b >> 13) & 0x3ff) as u16;
    sign | ((exp as u16) << 10) | man
}

fn f32_to_f24(v: f32) -> u32 {
    let b = v.to_bits();
    if v == 0.0 {
        return 0;
    }
    let sign = (b >> 31) << 23;
    let exp = (((b >> 23) & 0xff) as i32 - 127 + 63) as u32;
    sign | (exp << 16) | ((b >> 7) & 0xffff)
}

#[test]
fn floating_point_dngs() {
    let (w, h) = (12, 5);
    let vals: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 200) as f32 / 64.0).collect();
    for (bits, pred, stride_factor) in [(32u16, 3u16, 1usize), (32, 34894, 2), (32, 34895, 4), (16, 3, 1), (24, 3, 1), (24, 34894, 2)] {
        let bp = bits as usize / 8;
        let mut raw = base_ifd(w, h, bits, 1, true);
        raw.set(t::SAMPLE_FORMAT, Value::Short(vec![3]));
        raw.set(t::COMPRESSION, Value::Short(vec![compression::DEFLATE]));
        raw.set(t::PREDICTOR, Value::Short(vec![pred]));
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&fp_encode(&vals, bp, stride_factor, w), 6);
        raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![z] });
        let img = decode(&dng(raw, ByteOrder::Little)).unwrap();
        assert_eq!(img.data, RawData::F32(vals.clone()), "{bits}-bit predictor {pred}");
        assert_eq!(img.white, vec![1.0]);
    }
    // uncompressed half floats, big-endian file
    let mut raw = base_ifd(w, h, 16, 1, true);
    raw.set(t::SAMPLE_FORMAT, Value::Short(vec![3]));
    raw.set(t::COMPRESSION, Value::Short(vec![1]));
    let bytes: Vec<u8> = vals.iter().flat_map(|&v| f32_to_f16(v).to_be_bytes()).collect();
    raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![bytes] });
    assert_eq!(decode(&dng(raw, ByteOrder::Big)).unwrap().data, RawData::F32(vals));
}

#[test]
fn non_finite_float_samples_decode_as_levels() {
    // NaN samples in a float CFA DNG spread through any arithmetic downstream: the decoder hands on numbers only
    let (w, h) = (24, 20);
    let mut raw = synthetic(w, h, Some(Cfa::bayer("RGGB").unwrap()), 1);
    let mut vals: Vec<f32> = (0..w * h).map(|i| ((i * 37) % 200) as f32 / 256.0).collect();
    for (i, v) in vals.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        if (8..14).contains(&x) && (6..12).contains(&y) {
            *v = f32::NAN;
        } else if i % 11 == 0 {
            *v = if i % 2 == 0 { f32::INFINITY } else { f32::NEG_INFINITY };
        }
    }
    raw.data = RawData::F32(vals.clone());
    raw.black = BlackLevel::uniform(0.0);
    raw.white = vec![0.75];
    for compression in [DngCompression::Uncompressed, DngCompression::Deflate { tile: 16, half: false }] {
        let bytes = write_dng(&raw, &DngWriteOptions { compression, ..Default::default() }).unwrap();
        let img = decode(&bytes).unwrap();
        // NaN and −∞ read as black, +∞ as the white level; finite samples are kept bit-exactly
        let RawData::F32(got) = &img.data else { panic!("float data expected") };
        for (a, b) in vals.iter().zip(got) {
            let want = match *a {
                f32::INFINITY => 0.75,
                v if v.is_finite() => v,
                _ => 0.0,
            };
            assert_eq!(want.to_bits(), b.to_bits(), "{a} read as {b}");
        }
        assert!(got.iter().all(|v| v.is_finite()));
    }
}

#[test]
fn lj92_strips_and_levels_and_linearization() {
    let (w, h) = (20, 12);
    let px = pattern(w, h, 1, 12);
    let mut raw = base_ifd(w, h, 12, 1, true);
    raw.set(t::COMPRESSION, Value::Short(vec![compression::JPEG]));
    let strips: Vec<Vec<u8>> = px.chunks(w * 5).map(|c| craft_raw::ljpeg::encode(c, w, c.len() / w, 1, 12, 6, 0)).collect();
    raw.set_image(ImageData::Strips { rows_per_strip: 5, strips });
    let table: Vec<u16> = (0..4096u32).map(|i| (i * i / 4096) as u16).collect();
    raw.set(t::LINEARIZATION_TABLE, Value::Short(table.clone()));
    raw.set(t::BLACK_LEVEL_REPEAT_DIM, Value::Short(vec![2, 2]));
    raw.set(t::BLACK_LEVEL, Value::Rational(vec![(10, 1), (11, 1), (12, 1), (13, 1)]));
    raw.set(t::BLACK_LEVEL_DELTA_H, Value::SRational(vec![(1, 2); 16]));
    raw.set(t::BLACK_LEVEL_DELTA_V, Value::SRational(vec![(-1, 1); 10]));
    raw.set(t::WHITE_LEVEL, Value::Short(vec![4000]));
    raw.set(t::ACTIVE_AREA, Value::Long(vec![1, 2, 11, 18]));
    raw.set(t::DEFAULT_CROP_ORIGIN, Value::Rational(vec![(1, 1), (2, 1)]));
    raw.set(t::DEFAULT_CROP_SIZE, Value::Rational(vec![(12, 1), (6, 1)]));
    let img = decode(&dng(raw, ByteOrder::Little)).unwrap();
    let expect: Vec<u16> = px.iter().map(|&v| table[v as usize]).collect();
    assert_eq!(img.data, RawData::U16(expect.clone()));
    assert!(img.linearized);
    assert_eq!(img.active_area, Rect::new(2, 1, 16, 10));
    assert_eq!(img.crop, Rect::new(1, 2, 12, 6));
    assert_eq!(img.black.at(1, 1, 0, 1), 13.0 + 0.5 - 1.0);
    assert_eq!(img.white, vec![4000.0]);
}

#[test]
fn opcode_lists_round_trip() {
    let mut raw = synthetic(32, 24, Some(Cfa::bayer("RGGB").unwrap()), 1);
    let area = Area { top: 0, left: 0, bottom: 24, right: 32, plane: 0, planes: 1, row_pitch: 1, col_pitch: 1 };
    raw.opcodes.list2 = vec![Opcode::ScalePerRow { area, scales: vec![2.0; 24] }];
    raw.opcodes.list3 = vec![Opcode::FixVignetteRadial { k: [0.0; 5], center: [0.5, 0.5] }, Opcode::Unknown { id: 99, flags: 1, params: vec![] }];
    let bytes = write_dng(&raw, &DngWriteOptions::default()).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back.opcodes, raw.opcodes);
    // and the header-only probe reports the same lists
    assert_eq!(craft_raw::probe_info(&bytes).unwrap().opcodes, raw.opcodes);
}

#[test]
fn preview_and_probe() {
    let raw = synthetic(16, 16, Some(Cfa::bayer("RGGB").unwrap()), 1);
    let bytes = write_dng(&raw, &DngWriteOptions::default()).unwrap();
    // the raw tiles are lossless JPEG: never offered as a preview
    assert!(embedded_preview(&bytes).is_none());
    assert_eq!(probe(&bytes), Some(RawFormat::Dng));
}

/// storytold/lightcraft#212: a NEF converted to DNG came out dark and green. NEF white balance is vendor multipliers
/// (no colour matrix, no `AsShotNeutral`); the DNG must carry them as its as-shot neutral.
#[test]
fn vendor_white_balance_survives_dng_conversion() {
    let mut raw = synthetic(16, 16, Some(Cfa::bayer("RGGB").unwrap()), 1);
    raw.color = ColorData::default();
    raw.wb_multipliers = Some([2.25, 1.0, 1.7421875]);
    let back = decode(&write_dng(&raw, &DngWriteOptions::default()).unwrap()).unwrap();
    let neutral = back.color.as_shot_neutral.expect("the multipliers became AsShotNeutral");
    for (n, m) in neutral.iter().zip([2.25f64, 1.0, 1.7421875]) {
        assert!((n * m - 1.0).abs() < 1e-3, "{neutral:?}");
    }
}

#[test]
fn rejects_unsupported_and_broken() {
    let (w, h) = (8, 8);
    let mut raw = base_ifd(w, h, 8, 1, true);
    raw.set(t::COMPRESSION, Value::Short(vec![compression::LOSSY_JPEG]));
    raw.set_image(ImageData::Strips { rows_per_strip: 8, strips: vec![vec![0; 10]] });
    // lossy JPEG DNGs decode now; a strip that isn't a JPEG is reported as corrupt
    assert!(matches!(decode(&dng(raw, ByteOrder::Little)), Err(craft_raw::RawError::Corrupt(_))));
    let mut raw = base_ifd(w, h, 16, 1, true);
    raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 3, 1]));
    raw.set(t::COMPRESSION, Value::Short(vec![1]));
    raw.set_image(ImageData::Strips { rows_per_strip: 8, strips: vec![vec![0; 128]] });
    assert!(decode(&dng(raw, ByteOrder::Little)).is_err());
}

/// A Lightroom-style DNG: profile look tags in IFD 0, LinearRaw data in a SubIFD (storytold/lightcraft#138).
fn profile_dng(order: ByteOrder, extra: impl FnOnce(&mut IfdBuilder)) -> Vec<u8> {
    let (w, h) = (8, 6);
    let px = pattern(w, h, 3, 16);
    let mut raw = base_ifd(w, h, 16, 3, false);
    raw.set(t::COMPRESSION, Value::Short(vec![1]));
    raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![u16_bytes(&px, order)] });
    dng_with(raw, order, extra)
}

fn hsv_floats(h: usize, s: usize, v: usize, f: impl Fn(usize, usize, usize) -> [f32; 3]) -> Vec<f32> {
    (0..v).flat_map(|vi| (0..h).flat_map(move |hi| (0..s).map(move |si| (vi, hi, si)))).flat_map(|(vi, hi, si)| f(vi, hi, si)).collect()
}

#[test]
fn profile_look_tags_are_read_from_ifd0() {
    for order in [ByteOrder::Little, ByteOrder::Big] {
        let bytes = profile_dng(order, |ifd0| {
            ifd0.set(t::PROFILE_HUE_SAT_MAP_DIMS, Value::Long(vec![6, 3, 1]));
            ifd0.set(t::PROFILE_HUE_SAT_MAP_DATA_1, Value::Float(hsv_floats(6, 3, 1, |_, h, s| [h as f32, 1.0 + s as f32 * 0.1, 1.0])));
            ifd0.set(t::PROFILE_HUE_SAT_MAP_DATA_2, Value::Float(hsv_floats(6, 3, 1, |_, _, _| [0.0, 1.0, 1.0])));
            ifd0.set(t::PROFILE_LOOK_TABLE_DIMS, Value::Long(vec![4, 2, 3]));
            ifd0.set(t::PROFILE_LOOK_TABLE_DATA, Value::Float(hsv_floats(4, 2, 3, |v, _, _| [0.0, 1.0, 1.0 - v as f32 * 0.05])));
            ifd0.set(t::PROFILE_LOOK_TABLE_ENCODING, Value::Long(vec![1]));
            ifd0.set(t::PROFILE_TONE_CURVE, Value::Float(vec![0.0, 0.0, 0.25, 0.15, 0.5, 0.55, 1.0, 1.0]));
        });
        let img = decode(&bytes).unwrap();
        assert_eq!(img.cpp, 3);
        let p = &img.color.profile;
        let hsm = p.hue_sat_map[0].as_ref().unwrap();
        assert_eq!((hsm.hue_divisions, hsm.sat_divisions, hsm.val_divisions), (6, 3, 1));
        // value-major, hue-middle, saturation-minor: entry (h = 2, s = 1) is at 2·3 + 1
        assert_eq!(hsm.data[7], [2.0, 1.1, 1.0]);
        assert!(p.hue_sat_map[1].is_some());
        let look = p.look_table.as_ref().unwrap();
        assert_eq!((look.hue_divisions, look.sat_divisions, look.val_divisions, look.srgb_value), (4, 2, 3, true));
        assert_eq!(look.data[2 * 4 * 2], [0.0, 1.0, 0.9]);
        assert_eq!(p.tone_curve.as_ref().unwrap().points.len(), 4);
        // the header-only probe sees the same profile
        assert_eq!(craft_raw::probe_info(&bytes).unwrap().color.profile, *p);
        // our DNG writer keeps the look with the data (conversions, smart previews)
        let again = decode(&write_dng(&img, &DngWriteOptions::default()).unwrap()).unwrap();
        assert_eq!(again.color.profile, *p);
    }
}

#[test]
fn malformed_profile_look_tags_are_ignored() {
    let bytes = profile_dng(ByteOrder::Little, |ifd0| {
        // data count doesn't match the dimensions
        ifd0.set(t::PROFILE_HUE_SAT_MAP_DIMS, Value::Long(vec![6, 3, 1]));
        ifd0.set(t::PROFILE_HUE_SAT_MAP_DATA_1, Value::Float(vec![0.0, 1.0, 1.0]));
        // one saturation division is not allowed
        ifd0.set(t::PROFILE_LOOK_TABLE_DIMS, Value::Long(vec![2, 1, 1]));
        ifd0.set(t::PROFILE_LOOK_TABLE_DATA, Value::Float(vec![0.0, 1.0, 1.0, 0.0, 1.0, 1.0]));
        // decreasing inputs and a NaN
        ifd0.set(t::PROFILE_TONE_CURVE, Value::Float(vec![0.0, 0.0, 0.6, 0.5, 0.4, f32::NAN, 1.0, 1.0]));
    });
    let img = decode(&bytes).unwrap();
    assert!(img.color.profile.is_empty(), "{:?}", img.color.profile);
    // and without any of the tags there is nothing to apply
    let plain = decode(&profile_dng(ByteOrder::Little, |_| {})).unwrap();
    assert!(plain.color.profile.is_empty());
}

/// Lossless JPEG XL codestream of `px` (`w × h × cpp` interleaved), 16-bit unless `eight`.
fn jxl_encode(px: &[u16], w: usize, h: usize, cpp: usize, eight: bool) -> Vec<u8> {
    use zune_core::bit_depth::BitDepth;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::EncoderOptions;
    let cs = if cpp == 1 { ColorSpace::Luma } else { ColorSpace::RGB };
    let (bytes, depth): (Vec<u8>, _) = if eight {
        (px.iter().map(|&v| v as u8).collect(), BitDepth::Eight)
    } else {
        (px.iter().flat_map(|v| v.to_ne_bytes()).collect(), BitDepth::Sixteen)
    };
    let mut out = Vec::new();
    zune_jpegxl::JxlSimpleEncoder::new(&bytes, EncoderOptions::new(w, h, cs, depth)).encode(&mut out).unwrap();
    out
}

/// A bare codestream wrapped in the ISO-BMFF container (signature, `ftyp`, `jxlc`).
fn jxl_container(cs: &[u8]) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a, 0, 0, 0, 0x14];
    b.extend_from_slice(b"ftypjxl \0\0\0\0jxl ");
    b.extend_from_slice(&((cs.len() + 8) as u32).to_be_bytes());
    b.extend_from_slice(b"jxlc");
    b.extend_from_slice(cs);
    b
}

/// A tiled JPEG XL raw IFD of `px`. Edge tiles alternate between being coded at their in-image size
/// and padded to the nominal tile size (with junk the decoder must drop); tiles alternate between bare
/// codestreams and containers.
fn jxl_tiled(px: &[u16], w: usize, h: usize, cpp: usize, bits: u16, cfa: bool, tile: (usize, usize)) -> IfdBuilder {
    let (tw, th) = tile;
    let mut tiles = Vec::new();
    for ty in 0..h.div_ceil(th) {
        for tx in 0..w.div_ceil(tw) {
            let i = tiles.len();
            let (vw, vh) = (tw.min(w - tx * tw), th.min(h - ty * th));
            let (cw, ch) = if i % 2 == 0 { (tw, th) } else { (vw, vh) };
            let mut t = vec![bits.min(8) * 7; cw * ch * cpp];
            for y in 0..vh {
                for x in 0..vw {
                    for s in 0..cpp {
                        t[(y * cw + x) * cpp + s] = px[((ty * th + y) * w + tx * tw + x) * cpp + s];
                    }
                }
            }
            let cs = jxl_encode(&t, cw, ch, cpp, bits == 8);
            tiles.push(if i % 3 == 1 { jxl_container(&cs) } else { cs });
        }
    }
    let mut raw = base_ifd(w, h, bits, cpp, cfa);
    raw.set(t::COMPRESSION, Value::Short(vec![compression::JPEG_XL]));
    raw.set_image(ImageData::Tiles { tile_width: tw as u32, tile_height: th as u32, tiles });
    raw
}

#[test]
fn jpeg_xl_tiles_decode_bit_exactly() {
    let (w, h) = (37, 29);
    // LinearRaw 3 × 16-bit, the full code range
    let px = pattern(w, h, 3, 16);
    let bytes = dng(jxl_tiled(&px, w, h, 3, 16, false, (16, 16)), ByteOrder::Big);
    let img = decode(&bytes).unwrap();
    assert_eq!((img.width, img.height, img.cpp), (w, h, 3));
    assert_eq!(img.data, RawData::U16(px));
    assert_eq!(craft_raw::probe_info(&bytes).unwrap(), img.info());
    // CFA 1 × 8/12/14-bit
    for bits in [8u32, 12, 14] {
        let px = pattern(w, h, 1, bits);
        let img = decode(&dng(jxl_tiled(&px, w, h, 1, bits as u16, true, (16, 8)), ByteOrder::Little)).unwrap();
        assert_eq!(img.data, RawData::U16(px), "{bits} bits");
        assert_eq!(img.white, vec![((1u32 << bits) - 1) as f32]);
        assert_eq!(img.cfa.unwrap().name(), "GRBG");
    }
    // strips: a single JPEG XL strip per 8 rows (the last one shorter)
    let px = pattern(w, h, 1, 12);
    let strips = px.chunks(w * 8).map(|c| jxl_encode(c, w, c.len() / w, 1, false)).collect();
    let mut raw = base_ifd(w, h, 12, 1, true);
    raw.set(t::COMPRESSION, Value::Short(vec![compression::JPEG_XL]));
    raw.set_image(ImageData::Strips { rows_per_strip: 8, strips });
    assert_eq!(decode(&dng(raw, ByteOrder::Big)).unwrap().data, RawData::U16(px));
}

#[test]
fn malformed_jpeg_xl_tiles_are_errors() {
    use craft_raw::RawError;
    let (w, h) = (16, 16);
    let px = pattern(w, h, 1, 12);
    let good = jxl_encode(&px, w, h, 1, false);
    let one = |tile: Vec<u8>, cpp: usize| {
        let mut raw = base_ifd(w, h, 12, cpp, cpp == 1);
        raw.set(t::COMPRESSION, Value::Short(vec![compression::JPEG_XL]));
        raw.set_image(ImageData::Tiles { tile_width: w as u32, tile_height: h as u32, tiles: vec![tile] });
        dng(raw, ByteOrder::Little)
    };
    // garbage, every truncation, and flipped bytes: errors (or, for a truncation that only drops
    // trailing padding, the exact samples), never panics
    let mut garbage = vec![0xff, 0x0a];
    garbage.extend((0..400u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8));
    assert!(matches!(decode(&one(garbage, 1)), Err(RawError::Corrupt(_))));
    for n in 0..good.len() {
        let bytes = one(good[..n].to_vec(), 1);
        if let Ok(img) = decode(&bytes) {
            assert_eq!(img.data, RawData::U16(px.clone()), "truncated to {n}");
        }
        let _ = craft_raw::probe_info(&bytes);
    }
    for i in 0..good.len() {
        let mut b = good.clone();
        b[i] ^= 0x5a;
        let _ = decode(&one(b, 1));
    }
    // a tile of the wrong size, or the wrong plane count for the IFD
    let small = jxl_encode(&px[..w * 8], w, 8, 1, false);
    assert!(matches!(decode(&one(small, 1)), Err(RawError::Corrupt(_))));
    assert!(matches!(decode(&one(good.clone(), 3)), Err(RawError::Corrupt(_))));
    assert!(matches!(craft_raw::probe_info(&one(good, 3)), Err(RawError::Corrupt(_))));
}

/// One undecodable tile makes the whole image an error: it is never returned with a black tile.
#[test]
fn a_jpeg_xl_tile_that_fails_is_an_error_for_the_image() {
    use craft_raw::RawError;
    let (w, h) = (32, 16);
    let px = pattern(w, h, 1, 12);
    let tile = |x0: usize| -> Vec<u16> { (0..16).flat_map(|y| px[y * w + x0..y * w + x0 + 16].to_vec()).collect() };
    let two = |second: Vec<u8>| {
        let mut raw = base_ifd(w, h, 12, 1, true);
        raw.set(t::COMPRESSION, Value::Short(vec![compression::JPEG_XL]));
        raw.set_image(ImageData::Tiles { tile_width: 16, tile_height: 16, tiles: vec![jxl_encode(&tile(0), 16, 16, 1, false), second] });
        dng(raw, ByteOrder::Little)
    };
    assert_eq!(decode(&two(jxl_encode(&tile(16), 16, 16, 1, false))).unwrap().data, RawData::U16(px.clone()));
    // the second tile is cut short; the first one alone is not an image
    let good = jxl_encode(&tile(16), 16, 16, 1, false);
    assert!(matches!(decode(&two(good[..good.len() / 2].to_vec())), Err(RawError::Corrupt(_))));
    assert!(matches!(decode(&two(vec![0xff, 0x0a, 1, 2, 3])), Err(RawError::Corrupt(_))));
}

/// The decoder's memory is bounded by the tile: a grey tile that also codes an alpha channel needs
/// about twice what one plane may use, and is an error rather than an allocation the header asks
/// for. The same samples without the alpha channel decode.
#[test]
fn jpeg_xl_tiles_beyond_their_decode_budget_are_errors() {
    use craft_raw::RawError;
    use zune_core::bit_depth::BitDepth;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::EncoderOptions;
    let (w, h) = (512, 512);
    let grey: Vec<u16> = (0..w * h).map(|i| ((i % w) * 97 + (i / w) * 31) as u16 & 0x0fff).collect();
    let with_alpha: Vec<u8> = grey.iter().flat_map(|&v| [v, 4095 - v]).flat_map(u16::to_ne_bytes).collect();
    let mut tile = Vec::new();
    zune_jpegxl::JxlSimpleEncoder::new(&with_alpha, EncoderOptions::new(w, h, ColorSpace::LumaA, BitDepth::Sixteen)).encode(&mut tile).unwrap();
    let one = |tile: Vec<u8>| {
        let mut raw = base_ifd(w, h, 12, 1, true);
        raw.set(t::COMPRESSION, Value::Short(vec![compression::JPEG_XL]));
        raw.set_image(ImageData::Tiles { tile_width: w as u32, tile_height: h as u32, tiles: vec![tile] });
        dng(raw, ByteOrder::Little)
    };
    assert!(matches!(decode(&one(tile)), Err(RawError::Corrupt(_))));
    assert_eq!(decode(&one(jxl_encode(&grey, w, h, 1, false))).unwrap().data, RawData::U16(grey));
}

#[test]
fn jpeg_xl_preview_ifd_is_offered() {
    let raw = synthetic(16, 16, Some(Cfa::bayer("RGGB").unwrap()), 1);
    let RawData::U16(px) = &raw.data else { unreachable!() };
    let mut sub = base_ifd(16, 16, 14, 1, true);
    sub.set(t::COMPRESSION, Value::Short(vec![compression::JPEG_XL]));
    sub.set_image(ImageData::Tiles { tile_width: 16, tile_height: 16, tiles: vec![jxl_encode(px, 16, 16, 1, false)] });
    let preview = jxl_container(&jxl_encode(&[200; 4 * 2 * 3], 4, 2, 3, true));
    let bytes = dng_with(sub, ByteOrder::Big, |ifd0| {
        ifd0.set(t::COMPRESSION, Value::Short(vec![compression::JPEG_XL]));
        ifd0.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![preview.clone()] });
    });
    // the rendered RGB preview is offered whole; the (single-tile) CFA raw never is
    assert_eq!(embedded_preview(&bytes).unwrap(), preview);
    assert_eq!(decode(&bytes).unwrap().data, raw.data);
}

/// `ProfileGainTableMap` / `ProfileGainTableMap2` bytes in `order`: a `dims` = (V, H, N) grid with
/// spacing 0.5 and origin 0.25, green-only input weights, `v2` = (data type, gamma, gain min,
/// gain max), then `data` (the stored gains, already in `order`).
fn gain_map_tag(order: ByteOrder, dims: [u32; 3], v2: Option<(u32, f32, f32, f32)>, data: &[u8]) -> Vec<u8> {
    let be = order == ByteOrder::Big;
    let u = |v: u32| if be { v.to_be_bytes() } else { v.to_le_bytes() };
    let mut b = Vec::new();
    b.extend_from_slice(&u(dims[0]));
    b.extend_from_slice(&u(dims[1]));
    for v in [0.5f64, 0.5, 0.25, 0.25] {
        b.extend_from_slice(&if be { v.to_be_bytes() } else { v.to_le_bytes() });
    }
    b.extend_from_slice(&u(dims[2]));
    for w in [0.0f32, 1.0, 0.0, 0.0, 0.0] {
        b.extend_from_slice(&u(w.to_bits()));
    }
    if let Some((dt, gamma, min, max)) = v2 {
        for v in [dt, gamma.to_bits(), min.to_bits(), max.to_bits()] {
            b.extend_from_slice(&u(v));
        }
    }
    b.extend_from_slice(data);
    b
}

fn f32_bytes(v: &[f32], order: ByteOrder) -> Vec<u8> {
    v.iter().flat_map(|f| if order == ByteOrder::Big { f.to_be_bytes() } else { f.to_le_bytes() }).collect()
}

/// [`profile_dng`] with binary tags in the raw IFD (where Apple writes `ProfileGainTableMap`) and
/// in IFD 0.
fn gain_map_dng(order: ByteOrder, raw_tags: &[(u16, &[u8])], ifd0_tags: &[(u16, &[u8])]) -> Vec<u8> {
    let (w, h) = (8, 6);
    let px = pattern(w, h, 3, 16);
    let mut raw = base_ifd(w, h, 16, 3, false);
    raw.set(t::COMPRESSION, Value::Short(vec![1]));
    raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![u16_bytes(&px, order)] });
    for (tag, b) in raw_tags {
        raw.set(*tag, Value::Undefined(b.to_vec()));
    }
    dng_with(raw, order, |ifd0| {
        for (tag, b) in ifd0_tags {
            ifd0.set(*tag, Value::Undefined(b.to_vec()));
        }
    })
}

#[test]
fn gain_table_maps_follow_dng_precedence() {
    const V1: u16 = t::PROFILE_GAIN_TABLE_MAP;
    const V2: u16 = t::PROFILE_GAIN_TABLE_MAP_2;
    for order in [ByteOrder::Big, ByteOrder::Little] {
        let v1 = gain_map_tag(order, [2, 2, 4], None, &f32_bytes(&(1..=16).map(|i| i as f32 * 0.25).collect::<Vec<_>>(), order));
        // Apple's layout: version 1 in the raw IFD, in the file's byte order
        let apple = gain_map_dng(order, &[(V1, &v1[..])], &[]);
        let img = decode(&apple).unwrap();
        let map = img.color.profile.gain_table_map.clone().expect("gain table map");
        assert_eq!((map.points_v, map.points_h, map.points_n, map.gamma), (2, 2, 4, 1.0));
        assert_eq!((map.spacing_v, map.origin_h, map.weights[1]), (0.5, 0.25, 1.0));
        assert_eq!(map.gains[5], 1.5);
        assert_eq!(craft_raw::probe_info(&apple).unwrap().color.profile, img.color.profile);
        // our DNG writer keeps it
        let again = decode(&write_dng(&img, &DngWriteOptions::default()).unwrap()).unwrap();
        assert_eq!(again.color.profile.gain_table_map.as_ref(), Some(&map));
        // version 1 in IFD 0 (where DNG 1.7 says it was meant to go) is read too
        let in_ifd0 = decode(&gain_map_dng(order, &[], &[(V1, &v1[..])])).unwrap();
        assert_eq!(in_ifd0.color.profile.gain_table_map.as_ref(), Some(&map));
        // version 2 in IFD 0 supersedes version 1: 8-bit gains over 0.5..=1.5, gamma 2
        let v2 = gain_map_tag(order, [1, 1, 2], Some((0, 2.0, 0.5, 1.5)), &[0, 255]);
        let img = decode(&gain_map_dng(order, &[(V1, &v1[..])], &[(V2, &v2[..])])).unwrap();
        let map2 = img.color.profile.gain_table_map.clone().unwrap();
        assert_eq!((map2.points_n, map2.gamma, map2.gains.clone()), (2, 2.0, vec![0.5, 1.5]));
        // a gamma ≠ 1 survives our writer (as version 2)
        let again = decode(&write_dng(&img, &DngWriteOptions::default()).unwrap()).unwrap();
        assert_eq!(again.color.profile.gain_table_map, Some(map2));
        // a malformed version 2 (gamma outside 0.25..=4) falls back to version 1
        let bad_v2 = gain_map_tag(order, [1, 1, 2], Some((0, 9.0, 0.5, 1.5)), &[0, 255]);
        assert_eq!(decode(&gain_map_dng(order, &[(V1, &v1[..])], &[(V2, &bad_v2[..])])).unwrap().color.profile.gain_table_map, Some(map));
    }
}

#[test]
fn malformed_gain_table_maps_are_ignored() {
    let order = ByteOrder::Little;
    let ok = gain_map_tag(order, [1, 1, 2], None, &f32_bytes(&[1.0, 2.0], order));
    let cases: Vec<(&str, Vec<u8>, bool)> = vec![
        ("valid", ok.clone(), false),
        ("empty", vec![], false),
        ("header only", ok[..64].to_vec(), false),
        ("one byte short", ok[..ok.len() - 1].to_vec(), false),
        ("NaN gain", gain_map_tag(order, [1, 1, 2], None, &f32_bytes(&[1.0, f32::NAN], order)), false),
        ("negative gain", gain_map_tag(order, [1, 1, 2], None, &f32_bytes(&[1.0, -2.0], order)), false),
        ("huge dimensions", gain_map_tag(order, [u32::MAX, u32::MAX, u32::MAX], None, &f32_bytes(&[1.0, 2.0], order)), false),
        ("zero points", gain_map_tag(order, [1, 0, 2], None, &[]), false),
        ("unknown data type", gain_map_tag(order, [1, 1, 2], Some((9, 1.0, 0.0, 1.0)), &[0, 0]), true),
        ("NaN gain range", gain_map_tag(order, [1, 1, 2], Some((0, 1.0, f32::NAN, 1.0)), &[0, 255]), true),
    ];
    for (what, bytes, v2) in cases {
        let tag = if v2 { t::PROFILE_GAIN_TABLE_MAP_2 } else { t::PROFILE_GAIN_TABLE_MAP };
        let img = decode(&gain_map_dng(order, &[(tag, &bytes[..])], &[])).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(img.color.profile.gain_table_map.is_some(), what == "valid", "{what}");
    }
}

/// The map is read in the file's byte order, else big-endian (`craft_raw::gaintable`): a
/// big-endian payload in a little-endian file is read, a little-endian one in a big-endian file
/// is not.
#[test]
fn gain_table_map_byte_orders() {
    use ByteOrder::{Big, Little};
    let payload = |o: ByteOrder| gain_map_tag(o, [1, 1, 2], None, &f32_bytes(&[1.0, 2.0], o));
    for (file, data, read) in [(Little, Little, true), (Little, Big, true), (Big, Big, true), (Big, Little, false)] {
        let img = decode(&gain_map_dng(file, &[(t::PROFILE_GAIN_TABLE_MAP, &payload(data)[..])], &[])).unwrap();
        assert_eq!(img.color.profile.gain_table_map.map(|m| m.gains), read.then(|| vec![1.0, 2.0]), "{data:?} payload, {file:?} file");
    }
}

/// A semantic mask IFD (8-bit, uncompressed) named `name`.
fn mask_ifd(w: usize, h: usize, name: &str, px: Vec<u8>) -> IfdBuilder {
    let mut m = IfdBuilder::new();
    m.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![t::SUBFILE_SEMANTIC_MASK]));
    m.set(t::IMAGE_WIDTH, Value::Long(vec![w as u32]));
    m.set(t::IMAGE_LENGTH, Value::Long(vec![h as u32]));
    m.set(t::BITS_PER_SAMPLE, Value::Short(vec![8]));
    m.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
    m.set(t::PHOTOMETRIC, Value::Short(vec![photometric::MASK]));
    m.set(t::COMPRESSION, Value::Short(vec![1]));
    if !name.is_empty() {
        m.set(t::SEMANTIC_NAME, Value::Ascii(name.into()));
    }
    m.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![px] });
    m
}

/// Semantic masks: a cropped mask is placed by its `MaskSubArea` (zero outside it) and doesn't disturb the raw
/// decode.
#[test]
fn semantic_masks_are_read_and_placed() {
    use craft_raw::semantic::MaskSubArea;
    for order in [ByteOrder::Little, ByteOrder::Big] {
        let bytes = profile_dng(order, |ifd0| {
            // a 4 × 2 crop at (top 1, left 2) of an 8 × 4 mask over the 8 × 6 image
            let mut sky = mask_ifd(4, 2, "sky", vec![10, 20, 30, 40, 50, 60, 70, 255]);
            sky.set(t::MASK_SUB_AREA, Value::Long(vec![1, 2, 8, 4]));
            ifd0.add_sub_ifd(sky);
            let mut skin = mask_ifd(2, 2, "skin", vec![0, 255, 255, 0]);
            skin.set(t::SEMANTIC_INSTANCE_ID, Value::Ascii("person_a".into()));
            ifd0.add_sub_ifd(skin);
        });
        let raw = decode(&bytes).unwrap();
        assert_eq!((raw.width, raw.height, raw.cpp), (8, 6, 3), "the masks are not the raw image");
        let masks = craft_raw::semantic_masks(&bytes);
        assert_eq!(masks.len(), 2);
        let (sky, skin) = (&masks[0], &masks[1]);
        assert_eq!((sky.name.as_str(), sky.instance_id.as_deref(), sky.width, sky.height), ("sky", None, 4, 2));
        assert_eq!(sky.sub_area, Some(MaskSubArea { top: 1, left: 2, full_width: 8, full_height: 4 }));
        assert_eq!((sky.data[0], sky.data[7]), (10 * 257, 65535));
        assert_eq!((skin.name.as_str(), skin.instance_id.as_deref(), skin.sub_area), ("skin", Some("person_a"), None));
        // positions are in uncropped-mask pixels: zero outside the stored crop
        assert!((sky.value_at(2.5, 1.5) - 10.0 / 255.0).abs() < 1e-4);
        assert!((sky.value_at(5.5, 2.5) - 1.0).abs() < 1e-4);
        assert_eq!((sky.value_at(1.5, 1.5), sky.value_at(2.5, 0.5), sky.value_at(6.5, 1.5)), (0.0, 0.0, 0.0));
    }
}

/// Malformed semantic masks are skipped (never a panic), and so is the `MaskSubArea` of a mask it
/// doesn't fit; the raw image still decodes.
#[test]
fn malformed_semantic_masks_are_skipped() {
    let bytes = profile_dng(ByteOrder::Little, |ifd0| {
        ifd0.add_sub_ifd(mask_ifd(2, 2, "", vec![1, 2, 3, 4])); // no SemanticName
        let mut rgb = mask_ifd(2, 2, "rgb", vec![0; 12]);
        rgb.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
        rgb.set(t::BITS_PER_SAMPLE, Value::Short(vec![8, 8, 8]));
        ifd0.add_sub_ifd(rgb);
        let mut huge = mask_ifd(2, 2, "huge", vec![0; 4]);
        huge.set(t::IMAGE_WIDTH, Value::Long(vec![100_000]));
        huge.set(t::IMAGE_LENGTH, Value::Long(vec![100_000]));
        ifd0.add_sub_ifd(huge);
        let mut tiny = mask_ifd(2, 2, "tiny data", vec![0; 4]);
        tiny.set(t::IMAGE_WIDTH, Value::Long(vec![4000]));
        tiny.set(t::IMAGE_LENGTH, Value::Long(vec![3000]));
        ifd0.add_sub_ifd(tiny);
        let mut jxl = mask_ifd(2, 2, "unknown coding", vec![0; 4]);
        jxl.set(t::COMPRESSION, Value::Short(vec![60_000]));
        ifd0.add_sub_ifd(jxl);
        let mut zero = mask_ifd(2, 2, "zero size", vec![0; 4]);
        zero.set(t::IMAGE_WIDTH, Value::Long(vec![0]));
        ifd0.add_sub_ifd(zero);
        let mut short = mask_ifd(4, 4, "short strip", vec![200; 3]);
        short.set(t::MASK_SUB_AREA, Value::Long(vec![u32::MAX, u32::MAX, u32::MAX, u32::MAX]));
        ifd0.add_sub_ifd(short);
        let mut outside = mask_ifd(2, 2, "sub area too small", vec![255; 4]);
        outside.set(t::MASK_SUB_AREA, Value::Long(vec![3, 0, 2, 4]));
        ifd0.add_sub_ifd(outside);
    });
    assert_eq!(decode(&bytes).unwrap().cpp, 3);
    let masks = craft_raw::semantic_masks(&bytes);
    let names: Vec<&str> = masks.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["short strip", "sub area too small"]);
    assert!(masks.iter().all(|m| m.sub_area.is_none()), "sub areas the mask doesn't fit are ignored");
    for m in &masks {
        assert_eq!(m.data.len(), m.width * m.height);
    }
    // not a TIFF at all
    assert!(craft_raw::semantic_masks(b"not a dng").is_empty());
}

/// Only the first `MAX_MASKS` mask IFDs are decoded, malformed ones included, so a file full of
/// mask IFDs costs a bounded number of decodes: here the first `MAX_MASKS` can't be decoded, and
/// the valid masks after them are never looked at.
#[test]
fn semantic_mask_ifds_past_the_cap_are_not_decoded() {
    use craft_raw::semantic::MAX_MASKS;
    let with = |undecodable: usize, valid: usize| {
        profile_dng(ByteOrder::Little, |ifd0| {
            for i in 0..undecodable {
                let mut m = mask_ifd(2, 2, &format!("undecodable {i}"), vec![0; 4]);
                m.set(t::COMPRESSION, Value::Short(vec![60_000]));
                ifd0.add_sub_ifd(m);
            }
            for i in 0..valid {
                ifd0.add_sub_ifd(mask_ifd(2, 2, &format!("valid {i}"), vec![255; 4]));
            }
        })
    };
    let bytes = with(MAX_MASKS, 3 * MAX_MASKS);
    assert_eq!(decode(&bytes).unwrap().cpp, 3);
    assert!(craft_raw::semantic_masks(&bytes).is_empty());
    // one fewer undecodable mask leaves room for exactly one valid one
    let names: Vec<String> = craft_raw::semantic_masks(&with(MAX_MASKS - 1, 3 * MAX_MASKS)).into_iter().map(|m| m.name).collect();
    assert_eq!(names, ["valid 0"]);
}

/// `BaselineSharpness` is read from IFD 0; a zero, negative or absurd value is ignored.
#[test]
fn baseline_sharpness_is_read_when_plausible() {
    for (value, want) in [((3, 2), Some(1.5)), ((0, 1), None), ((1, 0), None), ((100, 1), None)] {
        let bytes = profile_dng(ByteOrder::Big, |ifd0| {
            ifd0.set(t::BASELINE_SHARPNESS, Value::Rational(vec![value]));
        });
        assert_eq!(decode(&bytes).unwrap().color.baseline_sharpness, want, "{value:?}");
    }
    let bytes = profile_dng(ByteOrder::Big, |ifd0| {
        ifd0.set(t::BASELINE_SHARPNESS, Value::SRational(vec![(-1, 1)]));
    });
    assert_eq!(decode(&bytes).unwrap().color.baseline_sharpness, None);
    assert_eq!(decode(&profile_dng(ByteOrder::Big, |_| {})).unwrap().color.baseline_sharpness, None);
}

/// The DNG CFAPattern is anchored at the top-left of the ActiveArea: a DNG whose area starts on an odd row or
/// column still has the pattern the tag states at the area's corner (storytold/lightcraft#1086), and the writer states it back the same way.
#[test]
fn cfa_pattern_is_relative_to_the_active_area() {
    let (w, h) = (20, 12);
    for (top, left, sensor_name) in [(0u32, 0u32, "GRBG"), (1, 0, "BGGR"), (0, 1, "RGGB"), (1, 1, "GBRG"), (3, 5, "GBRG"), (2, 4, "GRBG")] {
        let mut raw = base_ifd(w, h, 16, 1, true);
        raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![u16_bytes(&pattern(w, h, 1, 16), ByteOrder::Little)] });
        raw.set(t::WHITE_LEVEL, Value::Short(vec![65535]));
        raw.set(t::ACTIVE_AREA, Value::Long(vec![top, left, h as u32 - 1, w as u32 - 1]));
        let img = decode(&dng(raw, ByteOrder::Little)).unwrap();
        assert_eq!(img.cfa.as_ref().unwrap().name(), sensor_name, "sensor anchor for area at ({left}, {top})");
        let at_area = |r: &RawImage| r.cfa.as_ref().unwrap().shifted(r.active_area.x, r.active_area.y).name();
        assert_eq!(at_area(&img), "GRBG", "tag pattern for area at ({left}, {top})");
        let back = decode(
            &write_dng(
                &img,
                &DngWriteOptions { compression: DngCompression::Uncompressed, order: ByteOrder::Little, xmp: None, ..Default::default() },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(back.cfa.as_ref().unwrap().name(), sensor_name);
        assert_eq!(at_area(&back), "GRBG");
    }
}

/// Two samples a pixel are neither one colour nor RGB (storytold/lightcraft#1511): such data is refused with a
/// reason; one, three and four samples decode.
#[test]
fn two_samples_per_pixel_are_refused_not_a_panic() {
    let (w, h) = (8, 6);
    let file = |cpp: usize| {
        let mut raw = base_ifd(w, h, 16, cpp, false);
        raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![u16_bytes(&pattern(w, h, cpp, 16), ByteOrder::Little)] });
        dng(raw, ByteOrder::Little)
    };
    let err = decode(&file(2)).unwrap_err();
    assert!(matches!(err, craft_raw::RawError::Unsupported(ref m) if m.contains("2 samples")), "{err}");
    for cpp in [1, 3, 4] {
        assert_eq!(decode(&file(cpp)).unwrap().cpp, cpp);
    }
    // a raw image built with two samples does not validate, so it cannot be written either
    let raw = synthetic(16, 12, None, 2);
    assert!(matches!(raw.validate(), Err(craft_raw::RawError::Unsupported(_))));
    assert!(write_dng(&raw, &DngWriteOptions::default()).is_err());
}

/// DNGs LightCraft wrote before the writer anchored the pattern at the ActiveArea carry a bare "LightCraft" Software
/// tag and a sensor-origin pattern; they keep that reading, while the current writer marks its files differently.
#[test]
fn legacy_lightcraft_dng_keeps_the_sensor_anchored_pattern() {
    let (w, h) = (20, 12);
    let build = |software: &str| {
        let mut raw = base_ifd(w, h, 16, 1, true);
        raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![u16_bytes(&pattern(w, h, 1, 16), ByteOrder::Little)] });
        raw.set(t::WHITE_LEVEL, Value::Short(vec![65535]));
        raw.set(t::ACTIVE_AREA, Value::Long(vec![1, 0, h as u32 - 1, w as u32 - 1]));
        raw.set(t::SOFTWARE, Value::Ascii(software.into()));
        decode(&dng(raw, ByteOrder::Little)).unwrap()
    };
    assert_eq!(build("LightCraft").cfa.unwrap().name(), "GRBG");
    assert_eq!(build("Adobe Photoshop Lightroom").cfa.unwrap().name(), "BGGR");
    // the writer refuses to mark a new file as one of those
    let img = build("Adobe Photoshop Lightroom");
    assert!(write_dng(&img, &DngWriteOptions { software: "LightCraft".into(), ..Default::default() }).is_err());
    assert_ne!(DngWriteOptions::default().software, "LightCraft");
}

// ---------------------------------------------------------------- behaviour brought from the other two readers
//
// PhotoCraft's `photocraft-raw` and FilmCraft's `filmcraft-dng` read DNG too. Where they did something this
// reader (LightCraft's) did not, it was ported, with the test here.

/// An uncompressed 16-bit CFA raw IFD of `w` × `h` with a WhiteLevel, and its samples.
fn plain_cfa(w: usize, h: usize) -> (IfdBuilder, Vec<u16>) {
    let px = pattern(w, h, 1, 16);
    let mut raw = base_ifd(w, h, 16, 1, true);
    raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![u16_bytes(&px, ByteOrder::Little)] });
    raw.set(t::WHITE_LEVEL, Value::Short(vec![65535]));
    (raw, px)
}

/// PhotoCraft (storytold/photocraft#950): each default-crop tag has its own default (DNG 1.7.1: origin (0, 0), size the whole
/// image), so a file with only `DefaultCropSize` crops from the active area's corner. This reader wanted both
/// tags and ignored a size on its own.
#[test]
fn default_crop_size_without_origin_crops_from_the_active_area_origin() {
    let (mut raw, _) = plain_cfa(20, 12);
    raw.set(t::ACTIVE_AREA, Value::Long(vec![2, 4, 12, 20]));
    raw.set(t::DEFAULT_CROP_SIZE, Value::Long(vec![10, 6]));
    let img = decode(&dng(raw, ByteOrder::Little)).unwrap();
    assert_eq!(img.active_area, Rect::new(4, 2, 16, 10));
    assert_eq!(img.crop, Rect::new(0, 0, 10, 6));
    // an origin on its own keeps the default size, the whole area, clipped to what is left of it
    let (mut raw, _) = plain_cfa(20, 12);
    raw.set(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![3, 2]));
    assert_eq!(decode(&dng(raw, ByteOrder::Little)).unwrap().crop, Rect::new(3, 2, 17, 10));
}

/// A crop that starts outside the active area, is empty, or holds a value no pixel count can be (PhotoCraft
/// rejects the file for the last; this reader left an empty rectangle in `crop`) is the whole active area.
#[test]
fn an_unusable_default_crop_is_the_whole_active_area() {
    let whole = Rect::new(0, 0, 20, 12);
    let cases: [(Value, Value); 6] = [
        (Value::Long(vec![20, 0]), Value::Long(vec![4, 4])),
        (Value::Long(vec![0, 12]), Value::Long(vec![4, 4])),
        (Value::Double(vec![1e300, 0.0]), Value::Long(vec![4, 4])),
        (Value::Double(vec![f64::NAN, 0.0]), Value::Long(vec![4, 4])),
        (Value::Long(vec![0, 0]), Value::Long(vec![0, 4])),
        (Value::SRational(vec![(-1, 1), (0, 1)]), Value::Long(vec![4, 4])),
    ];
    for (origin, size) in cases {
        let (mut raw, _) = plain_cfa(20, 12);
        raw.set(t::DEFAULT_CROP_ORIGIN, origin.clone());
        raw.set(t::DEFAULT_CROP_SIZE, size.clone());
        assert_eq!(decode(&dng(raw, ByteOrder::Little)).unwrap().crop, whole, "{origin:?} {size:?}");
    }
    // a tag with one value instead of two
    let (mut raw, _) = plain_cfa(20, 12);
    raw.set(t::DEFAULT_CROP_SIZE, Value::Long(vec![4]));
    assert_eq!(decode(&dng(raw, ByteOrder::Little)).unwrap().crop, whole);
}

/// PhotoCraft: a WhiteLevel too large for the sample type (a DOUBLE past `f32::MAX`) casts to infinity, which
/// blacks the picture out. It is ignored, as a zero one is, and the default white stands.
#[test]
fn an_infinite_white_level_falls_back_to_the_default() {
    for white in [Value::Double(vec![1e300]), Value::Double(vec![f64::INFINITY]), Value::Double(vec![f64::NAN]), Value::Long(vec![0])] {
        let (mut raw, _) = plain_cfa(8, 6);
        raw.set(t::WHITE_LEVEL, white.clone());
        assert_eq!(decode(&dng(raw, ByteOrder::Little)).unwrap().white, vec![65535.0], "{white:?}");
    }
}

/// PhotoCraft: without a WhiteLevel, the white of a linearized image is the table's largest output, since the
/// level is in linearized values. An 8-bit file whose table expands to 16 bits clipped at 255 here.
#[test]
fn white_level_defaults_to_the_linearization_tables_maximum() {
    let (w, h) = (8, 6);
    let codes: Vec<u8> = (0..w * h).map(|i| (i * 5) as u8).collect();
    let table: Vec<u16> = (0..256u32).map(|i| (i * 200) as u16).collect();
    let build = |white: Option<u16>| {
        let mut raw = base_ifd(w, h, 8, 1, true);
        raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![codes.clone()] });
        raw.set(t::LINEARIZATION_TABLE, Value::Short(table.clone()));
        if let Some(white) = white {
            raw.set(t::WHITE_LEVEL, Value::Short(vec![white]));
        }
        decode(&dng(raw, ByteOrder::Little)).unwrap()
    };
    let img = build(None);
    assert!(img.linearized);
    assert_eq!(img.data, RawData::U16(codes.iter().map(|&c| table[c as usize]).collect()));
    assert_eq!(img.white, vec![51000.0], "255 × 200, the table's top");
    // a stated level still wins
    assert_eq!(build(Some(40000)).white, vec![40000.0]);
}

/// PhotoCraft: `AsShotWhiteXY` is a chromaticity, so both coordinates lie inside (0, 1).
#[test]
fn an_as_shot_white_outside_the_chromaticity_range_is_ignored() {
    let read = |xy: Vec<(u32, u32)>| {
        let (raw, _) = plain_cfa(8, 6);
        decode(&dng_with(raw, ByteOrder::Little, |ifd0| {
            ifd0.set(t::AS_SHOT_WHITE_XY, Value::Rational(xy));
        }))
        .unwrap()
        .color
        .as_shot_white_xy
    };
    assert_eq!(read(vec![(1, 4), (1, 2)]), Some([0.25, 0.5]));
    assert_eq!(read(vec![(3, 2), (1, 2)]), None);
    assert_eq!(read(vec![(1, 4), (1, 1)]), None);
    assert_eq!(read(vec![(0, 1), (1, 2)]), None);
}

/// PhotoCraft's test of an Orientation stored as a LONG past 65535: it is not truncated into the valid range
/// (65542 is not 6). craft-tiff's integer accessors saturate, so this already held; the test pins it.
#[test]
fn an_orientation_past_u16_is_not_truncated_into_range() {
    let read = |v: Value| {
        let (raw, _) = plain_cfa(8, 6);
        decode(&dng_with(raw, ByteOrder::Little, |ifd0| {
            ifd0.set(t::ORIENTATION, v);
        }))
        .unwrap()
        .orientation
    };
    assert_eq!(read(Value::Long(vec![6])), Orientation::Rotate90);
    assert_eq!(read(Value::Long(vec![65536 + 6])), Orientation::Normal);
    assert_eq!(read(Value::Short(vec![9])), Orientation::Normal);
}

/// A lossy DNG (compression 34892) around one baseline JPEG tile of `w` × `h` RGB at `bits` per sample.
fn lossy_dng(w: usize, h: usize, bits: u16, rgb: &[u8]) -> Vec<u8> {
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 100).encode(rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    let mut raw = base_ifd(w, h, bits, 3, false);
    raw.set(t::COMPRESSION, Value::Short(vec![compression::LOSSY_JPEG]));
    raw.set_image(ImageData::Tiles { tile_width: w as u32, tile_height: h as u32, tiles: vec![jpeg] });
    dng(raw, ByteOrder::Little)
}

/// Lossy DNG from a hand-built file (the tiles are YCbCr-coded baseline JPEGs of the stored R, G, B, as
/// converters write them): each channel comes back at its own level, with a LinearizationTable applied to the
/// stored 8-bit codes. PhotoCraft and FilmCraft: DNG stores lossy JPEG at 8 bits a sample, and another depth is
/// refused instead of reading 8-bit codes as deeper samples.
#[test]
fn lossy_jpeg_dng_decodes_stored_values_and_only_at_8_bits() {
    let (w, h) = (16, 16);
    let rgb: Vec<u8> = (0..w * h).flat_map(|_| [40u8, 120, 200]).collect();
    let img = decode(&lossy_dng(w, h, 8, &rgb)).unwrap();
    assert_eq!((img.width, img.height, img.cpp, img.bits), (w, h, 3, 8));
    assert_eq!(img.white, vec![255.0]);
    let RawData::U16(v) = &img.data else { panic!("integer samples") };
    for px in v.chunks(3) {
        for (got, want) in px.iter().zip([40i32, 120, 200]) {
            assert!((i32::from(*got) - want).abs() <= 3, "{px:?}");
        }
    }
    for bits in [12u16, 16] {
        for read in [craft_raw::decode(&lossy_dng(w, h, bits, &rgb)).map(|_| ()), craft_raw::probe_info(&lossy_dng(w, h, bits, &rgb)).map(|_| ())] {
            assert!(matches!(&read, Err(craft_raw::RawError::Unsupported(m)) if m.contains("lossy JPEG DNG")), "{bits} bits: {read:?}");
        }
    }
    // truncations of a lossy DNG never panic and never give a picture with a hole
    let file = lossy_dng(w, h, 8, &rgb);
    for n in 0..file.len() {
        if let Ok(cut) = decode(&file[..n]) {
            assert_eq!(cut.data.len(), w * h * 3);
        }
    }
}

/// FilmCraft: the CinemaDNG tags of a frame (FrameRate, TimeCodes, ReelName) are read, and written back.
#[test]
fn cinema_dng_tags_are_read_and_written() {
    use craft_raw::{CinemaTags, Timecode};
    let frame = |rate: Value, tc: Vec<u8>| {
        let (raw, _) = plain_cfa(8, 6);
        decode(&dng_with(raw, ByteOrder::Big, |ifd0| {
            ifd0.set(51044, rate);
            ifd0.set(51043, Value::Byte(tc));
            ifd0.set(51081, Value::Ascii("A001_C003".into()));
        }))
        .unwrap()
    };
    // 01:02:03:04, drop frame
    let img = frame(Value::SRational(vec![(24000, 1001)]), vec![0x44, 0x03, 0x02, 0x01, 0, 0, 0, 0]);
    let tc = Timecode { hours: 1, minutes: 2, seconds: 3, frames: 4, drop_frame: true };
    assert_eq!(img.cinema, CinemaTags { frame_rate: Some((24000, 1001)), timecode: Some(tc), reel_name: Some("A001_C003".into()) });
    assert_eq!(craft_raw::probe_info(&dng_with(plain_cfa(8, 6).0, ByteOrder::Little, |_| {})).unwrap().cinema, CinemaTags::default(), "a still");
    // the writer keeps them
    let back = decode(&write_dng(&img, &DngWriteOptions::default()).unwrap()).unwrap();
    assert_eq!(back.cinema, img.cinema);
    assert_eq!(Timecode::from_bytes(&tc.to_bytes()), Some(tc));
    // an implausible rate (zero, negative, above 1000 fps) and a time code that is not BCD are ignored
    for rate in
        [Value::SRational(vec![(0, 1)]), Value::SRational(vec![(-24, 1)]), Value::Rational(vec![(24, 0)]), Value::Rational(vec![(100_000, 1)])]
    {
        assert_eq!(frame(rate.clone(), vec![0; 8]).cinema.frame_rate, None, "{rate:?}");
    }
    for bad in [vec![0x0a, 0, 0, 0, 0, 0, 0, 0], vec![0, 0x60, 0, 0, 0, 0, 0, 0], vec![0, 0, 0, 0x24, 0, 0, 0, 0], vec![1, 2, 3]] {
        assert_eq!(frame(Value::Rational(vec![(25, 1)]), bad.clone()).cinema.timecode, None, "{bad:?}");
    }
}

// ---------------------------------------------------------------- limits

/// `Limits::max_samples`: an image one sample over is refused by the decode and by the header probe, before
/// anything is allocated; at the limit it decodes.
#[test]
fn the_sample_limit_is_enforced() {
    let (raw, px) = plain_cfa(20, 12);
    let file = dng(raw, ByteOrder::Little);
    let mut limits = craft_raw::Limits::default();
    limits.max_samples = 20 * 12;
    assert_eq!(craft_raw::decode_with(&file, &limits).unwrap().data, RawData::U16(px));
    limits.max_samples = 20 * 12 - 1;
    assert_eq!(craft_raw::decode_with(&file, &limits).err(), Some(craft_raw::RawError::Limit("image too large")));
    assert_eq!(craft_raw::probe_info_with(&file, &limits).err(), Some(craft_raw::RawError::Limit("image too large")));
    // a limit above the crate's own ceiling is held to it
    limits.max_samples = usize::MAX;
    assert!(craft_raw::decode_with(&file, &limits).is_ok());
}

/// `Limits::max_lossy_tile_samples`: a lossy tile's JPEG may not decode to more samples than this, whatever
/// the tile tags say.
#[test]
fn the_lossy_tile_limit_is_enforced() {
    let (w, h) = (16, 16);
    let rgb = vec![128u8; w * h * 3];
    let file = lossy_dng(w, h, 8, &rgb);
    let mut limits = craft_raw::Limits::default();
    limits.max_lossy_tile_samples = w * h * 3;
    assert!(craft_raw::decode_with(&file, &limits).is_ok());
    limits.max_lossy_tile_samples = w * h * 3 - 1;
    assert!(matches!(craft_raw::decode_with(&file, &limits), Err(craft_raw::RawError::Corrupt(m)) if m.contains("would exceed")));
}
