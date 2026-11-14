//! Photoshop's private TIFF tags (34377 image resources, 37724 ImageSourceData) travel through
//! `Metadata` as opaque bytes: written when present, read from either byte order and from
//! BigTIFF, and never a panic or an allocation on malformed input.

use crate::common;
use common::*;
use craft_image_codecs::*;

fn with_tags(layout: ChannelLayout, sample: SampleType) -> Image {
    let mut img = synth(9, 7, layout, sample, 5, 0.3);
    img.meta.photoshop_resources =
        Some(b"8BIM\x03\xed\x00\x00\x00\x00\x00\x10\x00\x48\x00\x00\x00\x01\x00\x01\x00\x48\x00\x00\x00\x01\x00\x01".to_vec());
    let mut layers = b"Adobe Photoshop Document Data Block\0".to_vec();
    layers.extend_from_slice(b"MIB8ryaL");
    layers.extend_from_slice(&4u32.to_le_bytes());
    layers.extend_from_slice(&[0, 0, 0, 0]);
    img.meta.photoshop_layers = Some(layers);
    img
}

#[test]
fn tags_round_trip_through_the_encoder() {
    for (layout, sample) in [
        (ChannelLayout::Rgb, SampleType::U8),
        (ChannelLayout::Rgba, SampleType::U16),
        (ChannelLayout::Gray, SampleType::F32),
        (ChannelLayout::CmykA, SampleType::U8),
    ] {
        for compression in [TiffCompression::None, TiffCompression::Deflate, TiffCompression::Lzw, TiffCompression::PackBits] {
            let img = with_tags(layout, sample);
            let bytes = encode(&img, Format::Tiff, &EncodeOptions { tiff_compression: compression, ..Default::default() }).unwrap();
            // The encoder writes the running target's byte order.
            assert_eq!(bytes.starts_with(b"II"), tiff_writes_little_endian());
            let (res, layers) = tiff_photoshop_tags(&bytes);
            assert_eq!(res, img.meta.photoshop_resources.as_deref());
            assert_eq!(layers, img.meta.photoshop_layers.as_deref());
            let back = decode(&bytes).unwrap();
            assert_eq!(back.meta.photoshop_resources, img.meta.photoshop_resources, "{layout:?} {sample:?} {compression:?}");
            assert_eq!(back.meta.photoshop_layers, img.meta.photoshop_layers);
            assert_eq!(back.data(), img.data());
        }
    }
}

#[test]
fn extra_samples_round_trip() {
    for (layout, sample) in [
        (ChannelLayout::Rgb, SampleType::U8),
        (ChannelLayout::Rgba, SampleType::U8),
        (ChannelLayout::Gray, SampleType::U16),
        (ChannelLayout::Cmyk, SampleType::F32),
    ] {
        let mut img = synth(4, 3, layout, sample, 2, 0.4);
        let n = 4 * 3;
        let b = sample.bytes();
        let data: Vec<u8> = (0..n * b).map(|i| (i * 17) as u8).collect();
        img.meta.extra_samples = vec![ExtraSample { kind: 0, data: data.clone() }];
        let bytes = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.layout(), layout, "{layout:?} {sample:?}");
        assert_eq!(back.data(), img.data(), "{layout:?} {sample:?}");
        assert_eq!(back.meta.extra_samples.len(), 1, "{layout:?} {sample:?}");
        assert_eq!(back.meta.extra_samples[0].kind, 0, "{layout:?} {sample:?}");
        assert_eq!(back.meta.extra_samples[0].data, data, "{layout:?} {sample:?}");
    }
}

/// Tall enough that the default strip size (about 1 MB) cannot hold every row, so a writer
/// that only fills the first strip, or predicts across the wrong sample count, cannot hide.
/// The spot plane is checked with the `tiff` crate, not PhotoCraft's decoder.
#[test]
fn multi_strip_spot_round_trip_and_dump() {
    let w = 800u32;
    let h = 400u32;
    let n = (w as usize) * (h as usize);
    for (layout, channels) in [(ChannelLayout::Rgb, 3usize), (ChannelLayout::Rgba, 4usize)] {
        let mut px = vec![0u8; n * channels];
        let mut spot = vec![0u8; n];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = y * w as usize + x;
                px[i * channels] = (x % 251) as u8;
                px[i * channels + 1] = (y % 251) as u8;
                px[i * channels + 2] = 40;
                if channels == 4 {
                    px[i * channels + 3] = if x > 20 && x < 780 && y > 10 && y + 10 < h as usize { 255 } else { 0 };
                }
                spot[i] = ((x * 3 + y * 17) % 251) as u8;
            }
        }
        let mut img = Image::from_u8(w, h, layout, px).unwrap();
        img.meta.extra_samples = vec![ExtraSample { kind: 0, data: spot.clone() }];
        for compression in [TiffCompression::None, TiffCompression::Deflate, TiffCompression::Lzw, TiffCompression::PackBits] {
            let bytes = encode(&img, Format::Tiff, &EncodeOptions { tiff_compression: compression, ..Default::default() }).unwrap();
            let label = format!("{layout:?} {compression:?}");
            let back = decode(&bytes).unwrap();
            assert_eq!(back.meta.extra_samples.len(), 1, "{label}");
            assert_eq!(back.meta.extra_samples[0].data, spot, "own decoder {label}");
            assert_eq!(back.data(), img.data(), "own decoder rgb {label}");
            let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
            let spp: u16 = dec.get_tag_unsigned(tiff::tags::Tag::SamplesPerPixel).unwrap();
            let bps = dec.get_tag_u16_vec(tiff::tags::Tag::BitsPerSample).unwrap();
            let extra = dec.get_tag_u16_vec(tiff::tags::Tag::ExtraSamples).unwrap();
            let rows = dec.find_tag_unsigned::<u32>(tiff::tags::Tag::RowsPerStrip).unwrap();
            assert_eq!(bps.len(), spp as usize, "{label}");
            assert_eq!(spp as usize, channels + 1, "{label}");
            assert_eq!(*extra.last().unwrap(), 0, "{label} spot ExtraSamples {extra:?}");
            assert!(rows.is_some_and(|r| r < h) || compression == TiffCompression::PackBits, "{label} rows {rows:?}");
            let patched = min_is_black(&bytes);
            let mut indep = tiff::decoder::Decoder::new(std::io::Cursor::new(patched.as_slice())).unwrap();
            let tiff::decoder::DecodingResult::U8(samples) = indep.read_image().unwrap() else { panic!("{label} not u8") };
            let spp = spp as usize;
            assert_eq!(samples.len(), n * spp, "{label}");
            let mut bad = 0usize;
            let mut first = None;
            for (i, want) in spot.iter().enumerate() {
                let got = samples.get(i * spp + spp - 1).copied();
                if got != Some(*want) {
                    bad += 1;
                    first.get_or_insert((i, got));
                }
            }
            assert_eq!(bad, 0, "{label} independent reader, first {first:?}");
        }
    }
}

/// Retag photometric interpretation 262 as MinIsBlack so `tiff`'s decoder returns every sample.
fn min_is_black(src: &[u8]) -> Vec<u8> {
    let mut bytes = src.to_vec();
    let le = bytes.first().copied() == Some(b'I');
    let ru16 = |b: &[u8], i: usize| -> Option<u16> {
        let s = b.get(i..i + 2)?;
        Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    };
    let ru32 = |b: &[u8], i: usize| -> Option<u32> {
        let s = b.get(i..i + 4)?;
        Some(if le { u32::from_le_bytes(s.try_into().ok()?) } else { u32::from_be_bytes(s.try_into().ok()?) })
    };
    let Some(ifd) = ru32(&bytes, 4) else { return bytes };
    let Some(n) = ru16(&bytes, ifd as usize) else { return bytes };
    for e in 0..n as usize {
        let at = ifd as usize + 2 + e * 12;
        if ru16(&bytes, at) == Some(262) {
            let value = if le { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
            if let Some(slot) = bytes.get_mut(at + 8..at + 10) {
                slot.copy_from_slice(&value);
            }
            break;
        }
    }
    bytes
}

#[test]
fn more_than_sixteen_extra_samples_is_refused() {
    let mut img = synth(2, 2, ChannelLayout::Rgb, SampleType::U8, 1, 0.0);
    img.meta.extra_samples = (0..MAX_EXTRA_SAMPLES + 1).map(|_| ExtraSample { kind: 0, data: vec![0; 4] }).collect();
    assert!(encode(&img, Format::Tiff, &EncodeOptions::default()).is_err());
}

#[test]
fn sixteen_extra_samples_round_trip() {
    for (layout, sample, base) in [
        (ChannelLayout::Rgb, SampleType::U8, 3usize),
        (ChannelLayout::Rgba, SampleType::U8, 4),
        (ChannelLayout::Gray, SampleType::U16, 1),
        (ChannelLayout::Cmyk, SampleType::F32, 4),
    ] {
        let mut img = synth(2, 2, layout, sample, 1, 0.0);
        let n = 2 * 2 * sample.bytes();
        img.meta.extra_samples = (0..MAX_EXTRA_SAMPLES).map(|k| ExtraSample { kind: 0, data: vec![k as u8; n] }).collect();
        let bytes = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.layout(), layout, "{layout:?} {sample:?} base {base}");
        assert_eq!(back.data(), img.data(), "{layout:?}");
        assert_eq!(back.meta.extra_samples.len(), MAX_EXTRA_SAMPLES, "{layout:?}");
        assert_eq!(back.meta.extra_samples[7].kind, 0, "{layout:?}");
        assert_eq!(back.meta.extra_samples[15].data, vec![15; n], "{layout:?}");
    }
}

/// An extra plane holds exactly one sample per pixel of the image's sample type. A plane of any
/// other length is refused rather than written: the writer copies one sample per pixel, so a
/// longer plane would lose its tail and a shorter one would run out mid-image.
#[test]
fn wrong_sized_extra_planes_are_refused() {
    let (w, h) = (2u32, 1u32);
    for (label, plane) in [("too long", vec![10u8, 20, 30]), ("too short", vec![10u8]), ("empty", Vec::new())] {
        let mut img = synth(w, h, ChannelLayout::Rgb, SampleType::U8, 1, 0.0);
        img.meta.extra_samples = vec![ExtraSample { kind: 0, data: plane.clone() }];
        let err = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap_err();
        assert!(matches!(err, CodecError::Encode { .. }), "{label}: {err}");
        assert!(err.to_string().contains("extra sample 0") && err.to_string().contains("exactly 2"), "{label}: {err}");
    }
    // The exact length still writes, and comes back unchanged.
    let plane = vec![10u8, 20];
    let mut img = synth(w, h, ChannelLayout::Rgb, SampleType::U8, 1, 0.0);
    img.meta.extra_samples = vec![ExtraSample { kind: 0, data: plane.clone() }];
    let back = decode(&encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap()).unwrap();
    assert_eq!(back.meta.extra_samples, vec![ExtraSample { kind: 0, data: plane }]);
}

/// The sample-type conversion an encode performs must not launder a wrong-sized plane: float Lab
/// is stored as CIELab at 16 bits, and `Image::convert` leaves a plane whose length does not match
/// the image alone, so a 7- or 9-byte plane for two pixels would otherwise reach the writer
/// reinterpreted at the new width.
#[test]
fn a_wrong_sized_plane_is_refused_before_a_sample_type_conversion() {
    let (w, h) = (2u32, 1u32);
    // Two pixels at 32-bit float are 8 bytes; the file's 16-bit samples are 4.
    for (label, len) in [("short", 7usize), ("long", 9)] {
        let mut img = synth(w, h, ChannelLayout::Lab, SampleType::F32, 1, 0.0);
        img.meta.extra_samples = vec![ExtraSample { kind: 0, data: vec![0; len] }];
        let err = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap_err();
        assert!(matches!(err, CodecError::Encode { .. }), "{label}: {err}");
        assert!(err.to_string().contains("exactly 8"), "{label}: {err}");
    }
}

/// Extra samples count against `max_alloc` with the image (their interleaved buffer and the
/// planes split from it): a budget that holds the RGB image but not sixteen extra planes opens
/// the image without them, with a warning, instead of allocating past it.
#[test]
fn extra_samples_count_against_the_allocation_limit() {
    // Big enough for several strips (about 1 MB each): one strip of all 19 samples fits the budget
    // of the image alone, as in real files.
    let (w, h) = (1024u32, 1024u32);
    let n = (w * h) as usize;
    let mut img = synth(w, h, ChannelLayout::Rgb, SampleType::U8, 1, 0.0);
    img.meta.extra_samples = (0..MAX_EXTRA_SAMPLES).map(|k| ExtraSample { kind: 0, data: vec![k as u8; n] }).collect();
    let bytes = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
    let with = |max_alloc: u64| decode_with(&bytes, &DecodeOptions { limits: Limits { max_alloc, ..Limits::default() }, ..DecodeOptions::default() });
    // RGB is 3 bytes a pixel; the extra samples add 2 x 16 more.
    let image_only = 3 * n as u64;
    let with_extras = (3 + 2 * MAX_EXTRA_SAMPLES as u64) * n as u64;
    let back = with(with_extras).expect("everything fits");
    assert_eq!(back.meta.extra_samples.len(), MAX_EXTRA_SAMPLES);
    assert!(back.warnings.is_empty());
    // The image fits, its extra samples don't: it opens as before they were read, with a warning.
    for max_alloc in [image_only, with_extras - 1] {
        let only = with(max_alloc).expect("the image alone fits");
        assert!(only.meta.extra_samples.is_empty());
        assert_eq!(only.warnings, vec![DecodeWarning::ExtraSamplesDropped { format: Format::Tiff, count: MAX_EXTRA_SAMPLES as u32 }]);
        assert_eq!(only.data(), back.data());
    }
    assert!(matches!(with(image_only - 1), Err(CodecError::LimitExceeded(_))), "the image itself must still fit");
}

/// The value field of a little-endian classic TIFF's ExtraSamples entry (tag 338, SHORT) holding
/// two values, so they sit in the entry itself.
fn extra_samples_values(bytes: &mut [u8]) -> &mut [u8] {
    assert_eq!(&bytes[..4], b"II*\0");
    let entry = [0x52, 0x01, 3, 0, 2, 0, 0, 0];
    let at = bytes.windows(8).position(|w| w == entry).expect("an ExtraSamples entry with two values");
    &mut bytes[at + 8..at + 12]
}

/// ExtraSamples values above 2 are reserved (TIFF 6.0, p. 31): they read as 0 (unspecified) and
/// are written as 0, so a save never carries a reserved value.
#[test]
fn reserved_extra_sample_kinds_read_as_unspecified() {
    let n = 4 * 4;
    let mut img = synth(4, 4, ChannelLayout::Rgb, SampleType::U8, 1, 0.0);
    img.meta.extra_samples = vec![ExtraSample { kind: 0, data: vec![7; n] }, ExtraSample { kind: 0, data: vec![9; n] }];
    let mut bytes = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
    // A file from another writer with reserved values 3 and 65535.
    let values = extra_samples_values(&mut bytes);
    assert_eq!(values, [0, 0, 0, 0]);
    values.copy_from_slice(&[3, 0, 0xFF, 0xFF]);
    let back = decode(&bytes).unwrap();
    assert_eq!(back.meta.extra_samples.iter().map(|e| e.kind).collect::<Vec<_>>(), [0, 0]);
    assert_eq!(back.layout(), ChannelLayout::Rgb, "a reserved value is no alpha");
    assert_eq!(back.meta.extra_samples[0].data, vec![7; n]);
    assert_eq!(back.meta.extra_samples[1].data, vec![9; n]);

    // An image that carries a reserved kind saves it as 0.
    img.meta.extra_samples[0].kind = 3;
    img.meta.extra_samples[1].kind = u16::MAX;
    let mut bytes = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
    assert_eq!(extra_samples_values(&mut bytes), [0, 0, 0, 0]);
}

/// Converting an image's sample type converts its extra samples with it, so they keep matching
/// the image: a 16-bit float TIFF's spot planes survive the 32-bit float a layered import uses.
#[test]
fn extra_samples_follow_a_sample_type_conversion() {
    let (w, h) = (3u32, 2u32);
    let n = (w * h) as usize;
    let values = [0.0f32, 0.25, 0.5, 0.75, 1.0, 0.125];
    let half = |v: f32| f16::from_f32(v).to_bits().to_ne_bytes();
    let rgb: Vec<u8> = (0..n * 3).flat_map(|_| half(0.5)).collect();
    let mut img = Image::from_raw(w, h, ChannelLayout::Rgb, SampleType::F16, rgb).unwrap();
    img.meta.extra_samples = vec![ExtraSample { kind: 0, data: values.iter().flat_map(|v| half(*v)).collect() }];

    let wide = img.convert(ChannelLayout::Rgb, SampleType::F32);
    let plane: Vec<f32> = wide.meta.extra_samples[0].data.as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes(*c)).collect();
    assert_eq!(plane, values);

    // To 8 bits, the plane quantizes exactly as a gray image of the same values does.
    let narrow = wide.convert(ChannelLayout::Rgb, SampleType::U8);
    let gray = Image::from_raw(w, h, ChannelLayout::Gray, SampleType::F32, wide.meta.extra_samples[0].data.clone()).unwrap();
    assert_eq!(narrow.meta.extra_samples[0].data, gray.convert(ChannelLayout::Gray, SampleType::U8).data());
    assert_eq!(narrow.meta.extra_samples[0].data.len(), n);
}

#[test]
fn absent_and_empty_tags_are_not_written() {
    let mut img = synth(4, 4, ChannelLayout::Rgb, SampleType::U8, 1, 0.0);
    let bytes = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
    assert_eq!(tiff_photoshop_tags(&bytes), (None, None));
    assert!(decode(&bytes).unwrap().meta.photoshop_layers.is_none());
    img.meta.photoshop_layers = Some(Vec::new());
    img.meta.photoshop_resources = Some(Vec::new());
    let bytes = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
    assert_eq!(tiff_photoshop_tags(&bytes), (None, None));
}

/// A minimal uncompressed 2×1 gray TIFF with a 37724 tag, in either byte order, classic or
/// BigTIFF: `payload` is stored inline when it fits the value field, at an offset otherwise.
fn hand_made(little: bool, big: bool, payload: &[u8]) -> Vec<u8> {
    let w16 = |v: u16| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
    let w32 = |v: u32| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
    let w64 = |v: u64| if little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() };
    let mut f = Vec::new();
    f.extend_from_slice(if little { b"II" } else { b"MM" });
    let header_len = if big { 16 } else { 8 };
    let pixels_at = header_len;
    let ifd_at = pixels_at + 2;
    f.extend(w16(if big { 43 } else { 42 }));
    if big {
        f.extend(w16(8));
        f.extend(w16(0));
        f.extend(w64(ifd_at as u64));
    } else {
        f.extend(w32(ifd_at as u32));
    }
    f.extend_from_slice(&[0x40, 0xc0]);
    // Entries: width, length, bits, compression, photometric, strip offsets, samples, rows per
    // strip, strip byte counts, 37724.
    let tags: [(u16, u16, u64, u64); 10] = [
        (256, 3, 1, 2),
        (257, 3, 1, 1),
        (258, 3, 1, 8),
        (259, 3, 1, 1),
        (262, 3, 1, 1),
        (273, 4, 1, pixels_at as u64),
        (277, 3, 1, 1),
        (278, 3, 1, 1),
        (279, 4, 1, 2),
        (37724, 7, payload.len() as u64, 0),
    ];
    let entry_len = if big { 20 } else { 12 };
    let inline = if big { 8 } else { 4 };
    let count_len = if big { 8 } else { 2 };
    let next_len = if big { 8 } else { 4 };
    let data_at = ifd_at + count_len + tags.len() * entry_len + next_len;
    let mut entries = Vec::new();
    for (tag, ty, count, value) in tags {
        entries.extend(w16(tag));
        entries.extend(w16(ty));
        entries.extend(if big { w64(count) } else { w32(count as u32) });
        let mut v = Vec::new();
        if tag == 37724 {
            if payload.len() <= inline {
                v.extend_from_slice(payload);
            } else {
                v.extend(if big { w64(data_at as u64) } else { w32(data_at as u32) });
            }
        } else if ty == 3 {
            v.extend(w16(value as u16));
        } else {
            v.extend(if big { w64(value) } else { w32(value as u32) });
        }
        v.resize(inline, 0);
        entries.extend(v);
    }
    f.extend(if big { w64(tags.len() as u64) } else { w16(tags.len() as u16) });
    f.extend(entries);
    f.extend(if big { w64(0) } else { w32(0) });
    assert_eq!(f.len(), data_at);
    if payload.len() > inline {
        f.extend_from_slice(payload);
    }
    f
}

#[test]
fn hand_made_files_in_every_layout() {
    let long = b"Adobe Photoshop Document Data Block\0MIB8".to_vec();
    for little in [false, true] {
        for big in [false, true] {
            for payload in [&b"ab"[..], &b"abcd"[..], &b"abcdefgh"[..], &long[..]] {
                let f = hand_made(little, big, payload);
                let (res, layers) = tiff_photoshop_tags(&f);
                assert_eq!(res, None);
                assert_eq!(layers, Some(payload), "little {little} big {big} len {}", payload.len());
                let img = decode(&f).unwrap();
                assert_eq!(img.dimensions(), (2, 1));
                assert_eq!(img.meta.photoshop_layers.as_deref(), Some(payload));
            }
        }
    }
}

#[test]
fn malformed_tags_read_as_absent_without_panicking() {
    let long = vec![7u8; 40];
    for little in [false, true] {
        for big in [false, true] {
            let f = hand_made(little, big, &long);
            // Truncated anywhere: never a panic; the payload is absent once it is cut.
            for cut in 0..f.len() {
                let (_, layers) = tiff_photoshop_tags(&f[..cut]);
                if cut < f.len() {
                    assert!(layers.is_none_or(|l| l.len() == 40));
                }
                let _ = decode(&f[..cut]);
            }
            // Bit flips in the directory.
            for i in 0..f.len() - 40 {
                let mut g = f.clone();
                g[i] ^= 0xff;
                let _ = tiff_photoshop_tags(&g);
                let _ = decode(&g);
            }
        }
    }
    assert_eq!(tiff_photoshop_tags(b""), (None, None));
    assert_eq!(tiff_photoshop_tags(b"II*\0"), (None, None));
    assert_eq!(tiff_photoshop_tags(b"MM\0*\xff\xff\xff\xff"), (None, None));
    // A count that overflows the file.
    let mut f = hand_made(true, false, &long);
    let at = f.windows(2).position(|w| w == 37724u16.to_le_bytes()).unwrap();
    f[at + 4..at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(tiff_photoshop_tags(&f).1, None);
}
