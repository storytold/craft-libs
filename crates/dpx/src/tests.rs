//! Tests ported from the three app decoders this crate replaces, plus limits and hostile input.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

/// Element fields of a test header.
#[derive(Clone, Copy)]
struct El {
    descriptor: u8,
    bits: u8,
    packing: u16,
}

fn el(descriptor: u8, bits: u8, packing: u16) -> El {
    El { descriptor, bits, packing }
}

/// A minimal DPX file: a 2048-byte header for `w` × `h` with element 0 set, then `data`.
fn file(big: bool, w: u32, h: u32, e: El, orientation: u16, data: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; HEADER_LEN];
    let p32 = |f: &mut Vec<u8>, at: usize, v: u32| f[at..at + 4].copy_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
    let p16 = |f: &mut Vec<u8>, at: usize, v: u16| f[at..at + 2].copy_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
    f[..4].copy_from_slice(if big { b"SDPX" } else { b"XPDS" });
    p32(&mut f, 4, HEADER_LEN as u32);
    p32(&mut f, 28, INDUSTRY_LEN);
    p16(&mut f, ORIENTATION, orientation);
    p16(&mut f, ORIENTATION + 2, 1);
    p32(&mut f, 772, w);
    p32(&mut f, 776, h);
    f[ELEMENT0 + 20] = e.descriptor;
    f[ELEMENT0 + 21] = UNDEF8;
    f[ELEMENT0 + 22] = UNDEF8;
    f[ELEMENT0 + 23] = e.bits;
    p16(&mut f, ELEMENT0 + 24, e.packing);
    p32(&mut f, ELEMENT0 + 28, UNDEF32);
    p32(&mut f, ELEMENT0 + 32, UNDEF32);
    f[1408..1664].fill(0xFF);
    f[FRAME_POSITION..1732].fill(0xFF);
    f[TIME_CODE..1972].fill(0xFF);
    f.extend_from_slice(data);
    f
}

fn words(big: bool, ws: &[u32]) -> Vec<u8> {
    ws.iter().flat_map(|w| if big { w.to_be_bytes() } else { w.to_le_bytes() }).collect()
}

fn dec(bytes: &[u8]) -> Frame {
    decode(bytes, &Limits::default()).unwrap()
}

/// Packing 0: components back to back, low bits first, each line on a 32-bit boundary.
fn packed(lines: &[&[u32]], bits: u32, big: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        let (mut acc, mut n) = (0u64, 0u32);
        let mut ws = Vec::new();
        for &v in *line {
            acc |= u64::from(v) << n;
            n += bits;
            while n >= 32 {
                ws.push(acc as u32);
                acc >>= 32;
                n -= 32;
            }
        }
        if n > 0 {
            ws.push(acc as u32);
        }
        out.extend(words(big, &ws));
    }
    out
}

#[test]
fn ten_bit_method_a_and_b_unpack_three_to_a_word() {
    for big in [true, false] {
        for (packing, base) in [(1u16, 22u32), (2, 20)] {
            let data = words(big, &[(1023 << base) | (512 << (base - 10)) | (1 << (base - 20))]);
            let f = dec(&file(big, 1, 1, el(50, 10, packing), 0, &data));
            assert_eq!(f.codes, [1023, 512, 1], "big {big} packing {packing}");
            assert_eq!(f.samples_u16(), [65535, 32800, 64]);
            assert_eq!((f.info.layout, f.info.bits, f.info.big_endian), (Layout::Rgb, 10, big));
        }
    }
}

#[test]
fn rgba_components_roll_across_ten_bit_words() {
    // 2 RGBA pixels = 8 components = 3 words; the last word is half empty.
    let comps = [1023u32, 0, 0, 1023, 0, 1023, 0, 0];
    let ws: Vec<u32> = comps.chunks(3).map(|c| c.iter().enumerate().fold(0, |w, (i, v)| w | (v << (22 - 10 * i)))).collect();
    let f = dec(&file(false, 2, 1, el(51, 10, 1), 0, &words(false, &ws)));
    assert_eq!(f.codes, [1023, 0, 0, 1023, 0, 1023, 0, 0]);
    assert!(f.info.layout.has_alpha());
}

#[test]
fn twelve_bit_methods_a_and_b() {
    let f = dec(&file(true, 2, 1, el(6, 12, 1), 0, &[0xFF, 0xF0, 0x00, 0x10]));
    assert_eq!(f.codes, [4095, 1]);
    assert_eq!(f.samples_u16(), [65535, 16]);
    // Method B: the value in the low 12 bits, whatever the padding bits hold.
    let f = dec(&file(true, 2, 1, el(6, 12, 2), 0, &[0xFF, 0xFF, 0x00, 0x01]));
    assert_eq!(f.codes, [4095, 1]);
}

#[test]
fn packed_ten_and_twelve_bit_components_fill_words_from_the_low_bits() {
    // 2 × 2 RGB 10-bit: 6 components (60 bits) a line, the fourth straddles two words.
    let lines: [&[u32]; 2] = [&[1023, 0, 512, 4, 1019, 1023], &[0, 1023, 0, 1023, 1023, 1023]];
    for big in [true, false] {
        let f = dec(&file(big, 2, 2, el(50, 10, 0), 0, &packed(&lines, 10, big)));
        assert_eq!(f.codes, [1023, 0, 512, 4, 1019, 1023, 0, 1023, 0, 1023, 1023, 1023], "big {big}");
    }
    // 3 × 1 RGB 12-bit: 9 components, 108 bits in 4 words.
    let line: [&[u32]; 1] = [&[4095, 0, 2048, 16, 4079, 0, 0, 0, 4095]];
    let f = dec(&file(true, 3, 1, el(50, 12, 0), 0, &packed(&line, 12, true)));
    assert_eq!(f.codes, [4095, 0, 2048, 16, 4079, 0, 0, 0, 4095]);
}

/// The bit order of packing 0 pinned to two independent readers: these are the image data words of
/// a 2 × 1 RGB frame that FFmpeg 9.0 (12-bit) and ImageMagick 7 (10- and 12-bit) both decode to
/// the code values asserted here.
#[test]
fn packed_bit_order_matches_other_readers() {
    let f = dec(&file(true, 2, 1, el(50, 12, 0), 0, &words(true, &[0x553F_F001, 0x000F_2AA1, 0x0000_0030])));
    assert_eq!(f.codes, [0x001, 0x3FF, 0x155, 0x2AA, 0x00F, 0x300]);
    let f = dec(&file(true, 2, 1, el(50, 10, 0), 0, &words(true, &[0x955F_FC01, 0x0C00_0FAA])));
    assert_eq!(f.codes, [0x001, 0x3FF, 0x155, 0x2AA, 0x00F, 0x300]);
}

/// A 10-bit luma element fills each word from its low end. The words are the image data of a
/// 6 × 1 file written by ImageMagick 7 from the code values asserted here, which FFmpeg 9.0 reads
/// back the same; FFmpeg and ImageMagick also read this crate's luma files back exactly.
#[test]
fn ten_bit_luma_fills_words_from_the_low_end() {
    let f = dec(&file(true, 6, 1, el(6, 10, 1), 0, &words(true, &[0x3206_4000, 0x7D19_04B0])));
    assert_eq!(f.codes, [0, 100, 200, 300, 400, 500]);
    // Method B: the same order, no padding below.
    let f = dec(&file(true, 4, 1, el(6, 10, 2), 0, &words(true, &[(3 << 20) | (2 << 10) | 1, 4])));
    assert_eq!(f.codes, [1, 2, 3, 4]);
    let bytes =
        encode(&Picture { width: 6, height: 1, layout: Layout::Luma, bits: 10, codes: &[0, 100, 200, 300, 400, 500] }, &Metadata::default()).unwrap();
    assert_eq!(&bytes[HEADER_LEN..], &words(true, &[0x3206_4000, 0x7D19_04B0])[..]);
}

#[test]
fn eight_and_sixteen_bit_lines_pad_to_words() {
    // 1 × 2 RGB: each 3-byte line padded to 4.
    let f = dec(&file(false, 1, 2, el(50, 8, 0), 3, &[1, 2, 3, 0, 4, 5, 6, 0]));
    assert_eq!(f.codes, [1, 2, 3, 4, 5, 6]);
    assert_eq!(f.samples_u16(), [257, 514, 771, 1028, 1285, 1542]);
    assert_eq!((f.info.orientation, f.info.flips(), f.info.exif_orientation()), (3, Some((true, true)), 3));
    for big in [true, false] {
        let f = dec(&file(big, 2, 1, el(6, 16, 0), 0, &if big { [0xFF, 0xFF, 0x80, 0x00] } else { [0xFF, 0xFF, 0x00, 0x80] }));
        assert_eq!(f.codes, [65535, 32768]);
        assert_eq!(f.samples_u16(), f.codes);
    }
}

#[test]
fn eight_and_sixteen_bit_lines_without_word_padding_still_open() {
    // 1 × 2 RGB: 3 bytes a line, 6 in all; padded lines would need 8.
    let f = dec(&file(true, 1, 2, el(50, 8, 0), 0, &[1, 2, 3, 4, 5, 6]));
    assert_eq!(f.codes, [1, 2, 3, 4, 5, 6]);
    // 1 × 2 luma 16-bit: 2 bytes a line.
    let f = dec(&file(true, 1, 2, el(6, 16, 0), 0, &[0, 1, 0, 2]));
    assert_eq!(f.codes, [1, 2]);
    // 10-bit lines are always whole words.
    assert!(decode(&file(true, 1, 2, el(50, 10, 1), 0, &[0; 6]), &Limits::default()).is_err());
}

#[test]
fn abgr_comes_back_as_rgba() {
    // 12-bit method A, stored A B G R.
    let f = dec(&file(true, 1, 1, el(52, 12, 1), 0, &[0x80, 0x00, 0x00, 0x00, 0x00, 0x10, 0xFF, 0xF0]));
    assert_eq!(f.codes, [4095, 1, 0, 2048]);
    assert_eq!((f.info.layout, f.info.descriptor), (Layout::Rgba, 52));
}

#[test]
fn ycbcr_descriptors_keep_stored_order_and_reference_codes() {
    for (descriptor, layout, n) in
        [(100, Layout::CbYCrY422, 2), (101, Layout::CbYACrYA4224, 3), (102, Layout::CbYCr444, 3), (103, Layout::CbYCrA4444, 4)]
    {
        let data: Vec<u8> = (0..(2 * n as u8).next_multiple_of(4)).collect();
        let mut f = file(true, 2, 1, el(descriptor, 8, 0), 0, &data);
        f[ELEMENT0 + 22] = 6;
        f[ELEMENT0 + 4..ELEMENT0 + 8].copy_from_slice(&16u32.to_be_bytes());
        f[ELEMENT0 + 12..ELEMENT0 + 16].copy_from_slice(&235u32.to_be_bytes());
        let frame = dec(&f);
        assert_eq!((frame.info.layout, frame.info.layout.components()), (layout, n));
        assert!(layout.is_ycbcr());
        assert_eq!(frame.codes, (0..2 * n as u16).collect::<Vec<_>>());
        assert_eq!((frame.info.colorimetric, frame.info.reference_low_code, frame.info.reference_high_code), (Some(6), 16, 235));
    }
    // 10-bit CbYCr filled, method A.
    let f = dec(&file(true, 1, 1, el(102, 10, 1), 0, &words(true, &[(512 << 22) | (64 << 12) | (940 << 2)])));
    assert_eq!(f.codes, [512, 64, 940]);
}

#[test]
fn end_of_line_padding_and_element_offset() {
    let mut f = file(true, 1, 2, el(6, 8, 0), 0, &[9, 9, 9, 9, 7, 0, 0, 0, 0xAA, 0xAA, 8]);
    // Element data 4 bytes past the file's image offset, 2 bytes of padding after each line.
    f[ELEMENT0 + 28..ELEMENT0 + 32].copy_from_slice(&(HEADER_LEN as u32 + 4).to_be_bytes());
    f[ELEMENT0 + 32..ELEMENT0 + 36].copy_from_slice(&2u32.to_be_bytes());
    // The last line needs neither its word padding nor end-of-line padding... but it does need its own word.
    assert!(decode(&f, &Limits::default()).is_err());
    f.extend_from_slice(&[0, 0, 0]);
    assert_eq!(dec(&f).codes, [7, 8]);
}

#[test]
fn orientation_codes_map_to_exif() {
    let mut f = file(true, 1, 1, el(6, 8, 0), 0, &[0; 4]);
    for (dpx, exif, flips) in [
        (0u16, 1u16, Some((false, false))),
        (1, 2, Some((true, false))),
        (2, 4, Some((false, true))),
        (3, 3, Some((true, true))),
        (4, 5, None),
        (5, 6, None),
        (6, 8, None),
        (7, 7, None),
        (0xFFFF, 1, Some((false, false))),
    ] {
        f[ORIENTATION..ORIENTATION + 2].copy_from_slice(&dpx.to_be_bytes());
        let info = probe(&f, &Limits::default()).unwrap();
        assert_eq!((info.exif_orientation(), info.flips()), (exif, flips), "DPX {dpx}");
    }
}

#[test]
fn header_fields_are_reported() {
    let mut f = file(true, 1, 1, el(50, 8, 0), 0, &[0; 4]);
    f[160..167].copy_from_slice(b" Maker ");
    f[260..264].copy_from_slice(b"Reel");
    f[460..464].copy_from_slice(b"(c)\x01");
    f[136..155].copy_from_slice(b"2026:01:02:03:04:05");
    f[ELEMENT0 + 21] = 3;
    f[ELEMENT0 + 22] = 1;
    f[770..772].copy_from_slice(&2u16.to_be_bytes());
    f[PIXEL_ASPECT..PIXEL_ASPECT + 8].copy_from_slice(&[0, 0, 0, 2, 0, 0, 0, 1]);
    f[FRAME_POSITION..FRAME_POSITION + 4].copy_from_slice(&42u32.to_be_bytes());
    f[FRAME_RATE..FRAME_RATE + 4].copy_from_slice(&24f32.to_bits().to_be_bytes());
    f[TIME_CODE..TIME_CODE + 4].copy_from_slice(&0x0102_0304u32.to_be_bytes());
    let info = probe(&f, &Limits::default()).unwrap();
    assert_eq!(info.creator.as_deref(), Some("Maker"));
    assert_eq!(info.project.as_deref(), Some("Reel"));
    assert_eq!(info.copyright.as_deref(), Some("(c)"));
    assert_eq!(info.creation_time.as_deref(), Some("2026:01:02:03:04:05"));
    assert_eq!((info.transfer, info.colorimetric, info.elements), (Some(3), Some(1), 2));
    assert_eq!(info.pixel_aspect, Some((2, 1)));
    assert_eq!((info.frame_position, info.frame_rate, info.time_code), (Some(42), Some(24.0), Some(0x0102_0304)));
    // Undefined fields are absent, and without an industry header its fields are not read.
    let mut bare = file(true, 1, 1, el(50, 8, 0), 0, &[0; 4]);
    let info = probe(&bare, &Limits::default()).unwrap();
    assert_eq!((info.transfer, info.colorimetric, info.pixel_aspect, info.frame_position, info.time_code), (None, None, None, None, None));
    assert_eq!((info.creator, info.frame_rate), (None, None));
    bare[FRAME_POSITION..FRAME_POSITION + 4].copy_from_slice(&42u32.to_be_bytes());
    bare[28..32].fill(0);
    assert_eq!(probe(&bare, &Limits::default()).unwrap().frame_position, None);
}

#[test]
fn widen_and_narrow_round_trip_every_code() {
    for bits in [8u8, 10, 12, 16] {
        for c in 0..=max_code(bits) {
            assert_eq!(narrow(widen(c, bits), bits), c, "{bits}-bit {c}");
        }
    }
    assert_eq!(widen(512, 10), (512.0f64 * 65535.0 / 1023.0).round() as u16);
    assert_eq!((widen(200, 8), widen(4095, 12), widen(u16::MAX, 10)), (200 * 257, 65535, 65535));
}

#[test]
fn time_codes_round_trip() {
    assert_eq!(format_time_code(0x0102_0304), "01:02:03:04");
    assert_eq!(parse_time_code("01:02:03:04"), Some(0x0102_0304));
    assert_eq!(parse_time_code(&format_time_code(0xABCD_0000)), Some(0xABCD_0000));
    assert_eq!(parse_time_code("1:2:3:4"), None);
}

#[test]
fn written_files_read_back_exactly() {
    for (layout, n) in [(Layout::Luma, 1usize), (Layout::Rgb, 3), (Layout::Rgba, 4)] {
        for bits in [8u8, 10, 12, 16] {
            // 5 × 3 so every depth has lines that need word padding.
            let codes: Vec<u16> = (0..5 * 3 * n).map(|i| (i as u16).wrapping_mul(2749) & max_code(bits)).collect();
            let pic = Picture { width: 5, height: 3, layout, bits, codes: &codes };
            let bytes = encode(&pic, &Metadata::default()).unwrap();
            assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as usize, bytes.len());
            let f = dec(&bytes);
            assert_eq!((f.info.width, f.info.height, f.info.layout, f.info.bits, f.info.kind), (5, 3, layout, bits, Kind::Dpx));
            assert_eq!(f.codes, codes, "{layout:?} {bits}");
            assert_eq!((f.info.transfer, f.info.creator.clone(), f.info.time_code, f.info.orientation), (None, None, None, 0));
            assert_eq!(f.info.reference_high_code, u32::from(max_code(bits)));
        }
    }
}

#[test]
fn written_metadata_reads_back() {
    let meta = Metadata {
        creator: Some("Maker"),
        project: Some("Réel"),
        copyright: Some("me"),
        creation_time: Some("2026:01:02:03:04:05"),
        transfer: Some(3),
        colorimetric: Some(1),
        frame_position: Some(7),
        frame_rate: Some(23.976),
        time_code: parse_time_code("10:20:30:12"),
    };
    let bytes = encode(&Picture { width: 1, height: 1, layout: Layout::Rgb, bits: 10, codes: &[1, 2, 3] }, &meta).unwrap();
    let i = dec(&bytes).info;
    assert_eq!((i.creator.as_deref(), i.project.as_deref(), i.copyright.as_deref()), (Some("Maker"), Some("R?el"), Some("me")));
    assert_eq!(i.creation_time.as_deref(), Some("2026:01:02:03:04:05"));
    assert_eq!((i.transfer, i.colorimetric, i.frame_position, i.frame_rate), (Some(3), Some(1), Some(7), Some(23.976)));
    assert_eq!(i.time_code.map(format_time_code).as_deref(), Some("10:20:30:12"));
}

#[test]
fn the_writer_refuses_what_it_cannot_store() {
    let pic = |layout, bits, codes| Picture { width: 1, height: 1, layout, bits, codes };
    let m = Metadata::default();
    assert!(matches!(encode(&pic(Layout::CbYCr444, 8, &[0; 3]), &m), Err(Error::Encode(_))));
    assert!(matches!(encode(&pic(Layout::Rgb, 9, &[0; 3]), &m), Err(Error::Encode(_))));
    assert!(matches!(encode(&pic(Layout::Rgb, 8, &[0; 4]), &m), Err(Error::Encode(_))));
    assert!(matches!(encode(&Picture { width: 0, height: 1, layout: Layout::Luma, bits: 8, codes: &[] }, &m), Err(Error::Encode(_))));
    assert!(matches!(encode_cineon(&pic(Layout::Rgba, 10, &[0; 4])), Err(Error::Encode(_))));
    assert!(matches!(encode_cineon(&pic(Layout::Rgb, 8, &[0; 3])), Err(Error::Encode(_))));
}

/// A minimal Cineon: a 1024-byte generic header, then `data`.
fn cineon(big: bool, w: u32, h: u32, channels: u8, bits: u8, packing: u8, data: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 1024];
    let p32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
    p32(&mut b, 0, 0x802A_5FD7);
    p32(&mut b, 4, 1024);
    b[193] = channels;
    for c in 0..usize::from(channels) {
        let base = CINEON_CHANNEL0 + 28 * c;
        b[base + 2] = bits;
        p32(&mut b, base + 4, w);
        p32(&mut b, base + 8, h);
    }
    b[681] = packing;
    b.extend_from_slice(data);
    b
}

fn ten(a: u32, c: u32, e: u32) -> u32 {
    (a << 22) | (c << 12) | (e << 2)
}

#[test]
fn cineon_reads_ten_bit_rgb_and_gray_in_both_byte_orders() {
    for big in [true, false] {
        let b = cineon(big, 2, 1, 3, 10, 5, &words(big, &[ten(0, 512, 1023), ten(1023, 0, 256)]));
        assert_eq!(sniff(&b), Some(Kind::Cineon));
        let f = dec(&b);
        assert_eq!((f.info.kind, f.info.width, f.info.height, f.info.layout, f.info.bits), (Kind::Cineon, 2, 1, Layout::Rgb, 10));
        assert_eq!(f.codes, [0, 512, 1023, 1023, 0, 256]);
        assert_eq!(f.samples_u16()[1], widen(512, 10));
        // Gray: three pixels a word; packing left unset reads as left justified.
        let f = dec(&cineon(big, 4, 1, 1, 10, 0, &words(big, &[ten(10, 20, 30), ten(40, 0, 0)])));
        assert_eq!((f.info.layout, f.codes.as_slice()), (Layout::Luma, &[10, 20, 30, 40][..]));
    }
    // Right justified (packing 6): the pad bits on top.
    let f = dec(&cineon(true, 1, 1, 3, 10, 6, &words(true, &[ten(1, 2, 3) >> 2])));
    assert_eq!(f.codes, [1, 2, 3]);
    // 8- and 16-bit samples.
    assert_eq!(dec(&cineon(true, 1, 1, 3, 8, 0, &[1, 2, 3, 0])).codes, [1, 2, 3]);
    assert_eq!(dec(&cineon(true, 2, 1, 1, 16, 0, &[1, 2, 3, 4])).codes, [0x0102, 0x0304]);
}

#[test]
fn cineon_orientation_codes_are_translated() {
    let mut b = cineon(true, 1, 1, 1, 10, 5, &[0; 4]);
    // Cineon 1 is bottom to top (DPX 2), Cineon 2 is right to left (DPX 1).
    for (code, dpx, flips) in [
        (0u8, 0u16, Some((false, false))),
        (1, 2, Some((false, true))),
        (2, 1, Some((true, false))),
        (3, 3, Some((true, true))),
        (5, 5, None),
        (255, 255, Some((false, false))),
    ] {
        b[192] = code;
        let info = probe(&b, &Limits::default()).unwrap();
        assert_eq!((info.orientation, info.flips()), (dpx, flips), "Cineon {code}");
    }
}

#[test]
fn cineon_refuses_what_it_does_not_read() {
    let good = cineon(true, 4, 4, 3, 10, 5, &[0; 64]);
    assert!(decode(&good, &Limits::default()).is_ok());
    let with = |at: usize, v: u8| {
        let mut b = good.clone();
        b[at] = v;
        decode(&b, &Limits::default())
    };
    assert!(matches!(with(198, 12), Err(Error::Unsupported(_))), "12-bit");
    assert!(matches!(with(193, 2), Err(Error::Unsupported(_))), "two channels");
    assert!(matches!(with(CINEON_CHANNEL0 + 28 + 2, 8), Err(Error::Unsupported(_))), "mixed depths");
    assert!(matches!(with(680, 1), Err(Error::Unsupported(_))), "line interleave");
    assert!(matches!(with(681, 1), Err(Error::Unsupported(_))), "byte packing");
    assert!(matches!(with(682, 1), Err(Error::Unsupported(_))), "signed");
    for cut in [0, 4, 200, 687, 1024, good.len() - 1] {
        assert!(decode(&good[..cut], &Limits::default()).is_err(), "cut {cut}");
    }
}

#[test]
fn written_cineon_reads_back_exactly() {
    for (layout, n) in [(Layout::Luma, 1usize), (Layout::Rgb, 3)] {
        let codes: Vec<u16> = (0..5 * 3 * n).map(|i| (i as u16).wrapping_mul(2749) & 1023).collect();
        let bytes = encode_cineon(&Picture { width: 5, height: 3, layout, bits: 10, codes: &codes }).unwrap();
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize, bytes.len());
        let f = dec(&bytes);
        assert_eq!((f.info.kind, f.info.width, f.info.height, f.info.layout, f.info.orientation), (Kind::Cineon, 5, 3, layout, 0));
        assert_eq!(f.codes, codes);
    }
}

#[test]
fn limits_are_checked_before_allocating() {
    // A header asking for 2^64 pixels with no data.
    let huge = file(true, u32::MAX, u32::MAX, el(50, 10, 1), 0, &[]);
    assert_eq!(decode(&huge, &Limits::default()), Err(Error::LimitExceeded("pixel count")));
    assert_eq!(probe(&huge, &Limits::default()), Err(Error::LimitExceeded("pixel count")));
    // Exactly at the limit passes, one over does not.
    let f = file(true, 2, 2, el(6, 8, 0), 0, &[0; 8]);
    assert!(decode(&f, &Limits { max_pixels: 4 }).is_ok());
    assert_eq!(decode(&f, &Limits { max_pixels: 3 }), Err(Error::LimitExceeded("pixel count")));
    // Within the limit but without the data: refused before the sample buffer is allocated.
    assert_eq!(
        decode(&file(true, 16384, 16384, el(50, 10, 1), 0, &[0; 64]), &Limits::default()),
        Err(Error::Malformed("truncated image data".into()))
    );
    assert_eq!(Limits::default().max_pixels, 1 << 28);
}

#[test]
fn hostile_headers_fail_cleanly() {
    let d = |f: &[u8]| decode(f, &Limits::default());
    assert_eq!(d(b"not a dpx"), Err(Error::NotDpx));
    assert_eq!(sniff(b"not a dpx"), None);
    assert_eq!((sniff(b"SDPX"), sniff(b"XPDS")), (Some(Kind::Dpx), Some(Kind::Dpx)));
    assert!(matches!(d(b"SDPX"), Err(Error::Malformed(_))));
    assert!(matches!(d(&file(true, 2, 2, el(50, 10, 1), 0, &[0; 12])), Err(Error::Malformed(_))), "truncated data");
    assert!(matches!(d(&file(true, 0, 1, el(50, 8, 0), 0, &[0; 4])), Err(Error::Malformed(_))), "zero width");
    assert!(matches!(d(&file(true, 1, 0, el(50, 8, 0), 0, &[0; 4])), Err(Error::Malformed(_))), "zero height");
    assert!(matches!(d(&file(true, 1, 1, el(150, 8, 0), 0, &[0; 4])), Err(Error::Unsupported(_))), "descriptor");
    assert!(matches!(d(&file(true, 1, 1, el(50, 10, 3), 0, &[0; 4])), Err(Error::Unsupported(_))), "packing");
    assert!(matches!(d(&file(true, 1, 1, el(50, 32, 0), 0, &[0; 16])), Err(Error::Unsupported(_))), "float");
    assert!(matches!(d(&file(true, 1, 1, el(50, 1, 0), 0, &[0; 16])), Err(Error::Unsupported(_))), "1-bit");
    assert!(d(&file(true, 3, 3, el(100, 12, 0), 0, &[0; 20])).is_err());
}

#[test]
fn hostile_element_fields_fail_cleanly() {
    let d = |f: &[u8]| decode(f, &Limits::default());
    let good = file(true, 4, 4, el(50, 10, 1), 0, &[0x5A; 64]);
    assert!(d(&good).is_ok());
    let with = |at: usize, v: &[u8]| {
        let mut b = good.clone();
        b[at..at + v.len()].copy_from_slice(v);
        d(&b)
    };
    assert!(matches!(with(ELEMENT0 + 26, &1u16.to_be_bytes()), Err(Error::Unsupported(_))), "run-length encoding");
    assert!(matches!(with(ELEMENT0, &1u32.to_be_bytes()), Err(Error::Unsupported(_))), "signed");
    assert!(matches!(with(770, &0u16.to_be_bytes()), Err(Error::Malformed(_))), "no elements");
    assert!(matches!(with(770, &9u16.to_be_bytes()), Err(Error::Malformed(_))), "too many elements");
    assert!(matches!(with(ELEMENT0 + 28, &(u32::MAX - 1).to_be_bytes()), Err(Error::Malformed(_))), "element offset past the end");
    assert!(matches!(with(4, &u32::MAX.to_be_bytes()), Err(Error::Malformed(_))), "image offset past the end");
    assert!(matches!(with(ELEMENT0 + 32, &(u32::MAX - 1).to_be_bytes()), Err(Error::Malformed(_))), "line padding past the end");
    // One line needs no end-of-line padding after it, however large.
    let mut one = file(true, 1, 1, el(50, 8, 0), 0, &[0; 4]);
    one[ELEMENT0 + 32..ELEMENT0 + 36].copy_from_slice(&(u32::MAX - 1).to_be_bytes());
    assert!(d(&one).is_ok());
}

/// Every prefix of a valid file and every single-byte change to its header either decodes or
/// errors; neither panics nor allocates past the limit.
#[test]
fn truncation_and_mutation_never_panic() {
    let limits = Limits::default();
    let dpx = encode(&Picture { width: 3, height: 2, layout: Layout::Rgba, bits: 10, codes: &[5; 24] }, &Metadata::default()).unwrap();
    let cin = encode_cineon(&Picture { width: 3, height: 2, layout: Layout::Rgb, bits: 10, codes: &[5; 18] }).unwrap();
    for good in [dpx, cin] {
        for n in 0..good.len() {
            assert!(decode(&good[..n], &limits).is_err(), "prefix {n}");
        }
        for at in 0..1024 {
            for v in [0u8, 1, 2, 0x7F, 0x80, 0xFF] {
                let mut b = good.clone();
                b[at] = v;
                let _ = probe(&b, &limits);
                if let Ok(f) = decode(&b, &limits) {
                    assert_eq!(f.codes.len(), f.info.width as usize * f.info.height as usize * f.info.layout.components());
                    assert!(f.codes.iter().all(|&c| c <= f.info.max_code()));
                }
            }
        }
    }
}
