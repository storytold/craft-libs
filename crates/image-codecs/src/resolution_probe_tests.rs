#![allow(clippy::unwrap_used)]

use super::{MAX_PPI, file_resolution, usable};
use crate::{ChannelLayout, EncodeOptions, Format, Image, SampleType};

/// A little- or big-endian classic TIFF whose IFD0 holds XResolution and YResolution
/// (`x`/1, `y`/1) and, when given, ResolutionUnit.
fn tiff(le: bool, x: u32, y: u32, unit: Option<u16>) -> Vec<u8> {
    let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let entries: u16 = if unit.is_some() { 3 } else { 2 };
    let rationals = 8 + 2 + 12 * u32::from(entries) + 4;
    let mut t = Vec::new();
    t.extend_from_slice(if le { b"II*\0" } else { b"MM\0*" });
    t.extend_from_slice(&u32b(8));
    t.extend_from_slice(&u16b(entries));
    for (tag, offset) in [(0x011Au16, rationals), (0x011B, rationals + 8)] {
        t.extend_from_slice(&u16b(tag));
        t.extend_from_slice(&u16b(5));
        t.extend_from_slice(&u32b(1));
        t.extend_from_slice(&u32b(offset));
    }
    if let Some(unit) = unit {
        t.extend_from_slice(&u16b(0x0128));
        t.extend_from_slice(&u16b(3));
        t.extend_from_slice(&u32b(1));
        t.extend_from_slice(&u16b(unit));
        t.extend_from_slice(&[0, 0]);
    }
    t.extend_from_slice(&u32b(0));
    for v in [x, 1, y, 1] {
        t.extend_from_slice(&u32b(v));
    }
    t
}

fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut s = vec![0xFF, marker];
    s.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
    s.extend_from_slice(payload);
    s
}

fn jfif(units: u8, x: u16, y: u16) -> Vec<u8> {
    let mut p = b"JFIF\0\x01\x02".to_vec();
    p.push(units);
    p.extend_from_slice(&x.to_be_bytes());
    p.extend_from_slice(&y.to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    segment(0xE0, &p)
}

fn exif_app1(tiff: &[u8]) -> Vec<u8> {
    let mut p = b"Exif\0\0".to_vec();
    p.extend_from_slice(tiff);
    segment(0xE1, &p)
}

fn jpeg(segments: &[Vec<u8>]) -> Vec<u8> {
    let mut j = vec![0xFF, 0xD8];
    for s in segments {
        j.extend_from_slice(s);
    }
    // Start of scan, then entropy data holding bytes that look like an APP0 marker.
    j.extend_from_slice(&segment(0xDA, &[1, 1, 0, 0, 63, 0]));
    j.extend_from_slice(&jfif(1, 9, 9));
    j.extend_from_slice(&[0xFF, 0xD9]);
    j
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    c.extend_from_slice(kind);
    c.extend_from_slice(data);
    c.extend_from_slice(&[0, 0, 0, 0]); // the CRC is not checked by a header probe
    c
}

fn png(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
    p.extend_from_slice(&png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]));
    for c in chunks {
        p.extend_from_slice(c);
    }
    p.extend_from_slice(&png_chunk(b"IEND", &[]));
    p
}

fn phys(x: u32, y: u32, unit: u8) -> Vec<u8> {
    let mut d = x.to_be_bytes().to_vec();
    d.extend_from_slice(&y.to_be_bytes());
    d.push(unit);
    png_chunk(b"pHYs", &d)
}

fn bmp(header_size: u32, x: i32, y: i32) -> Vec<u8> {
    let mut b = vec![0u8; 54];
    b[..2].copy_from_slice(b"BM");
    b[14..18].copy_from_slice(&header_size.to_le_bytes());
    b[38..42].copy_from_slice(&x.to_le_bytes());
    b[42..46].copy_from_slice(&y.to_le_bytes());
    b
}

fn close(got: Option<(f32, f32)>, want: (f32, f32)) {
    let (x, y) = got.unwrap();
    assert!((x - want.0).abs() < 0.05 && (y - want.1).abs() < 0.05, "{x} {y} vs {want:?}");
}

#[test]
fn jpeg_jfif_density_in_inches_and_centimetres() {
    assert_eq!(file_resolution(&jpeg(&[jfif(1, 300, 150)])), Some((300.0, 150.0)));
    close(file_resolution(&jpeg(&[jfif(2, 100, 100)])), (254.0, 254.0));
    // Units 0 is an aspect ratio, not a density.
    assert_eq!(file_resolution(&jpeg(&[jfif(0, 1, 1)])), None);
    // Nothing in the header: the APP0 look-alike after the start of scan is not read.
    assert_eq!(file_resolution(&jpeg(&[])), None);
}

#[test]
fn jpeg_exif_wins_over_jfif_and_non_exif_app1_is_skipped() {
    let xmp = segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>");
    let exif = exif_app1(&tiff(false, 600, 600, Some(2)));
    assert_eq!(file_resolution(&jpeg(&[jfif(1, 72, 72), xmp.clone(), exif])), Some((600.0, 600.0)));
    // An unusable EXIF value falls back to JFIF.
    let zero = exif_app1(&tiff(true, 0, 0, None));
    assert_eq!(file_resolution(&jpeg(&[jfif(1, 96, 96), xmp.clone(), zero])), Some((96.0, 96.0)));
    let huge = exif_app1(&tiff(true, 1_000_000, 1_000_000, None));
    assert_eq!(file_resolution(&jpeg(&[jfif(1, 96, 96), huge])), Some((96.0, 96.0)));
    // Sub-1.0 EXIF rational also falls back to JFIF.
    let mut sub = tiff(true, 1, 1, None);
    let len = sub.len();
    sub[len - 12..len - 8].copy_from_slice(&4u32.to_le_bytes());
    sub[len - 4..len].copy_from_slice(&4u32.to_le_bytes());
    assert_eq!(file_resolution(&jpeg(&[jfif(1, 96, 96), xmp, exif_app1(&sub)])), Some((96.0, 96.0)));
}

#[test]
fn jpeg_fill_bytes_and_standalone_markers_are_stepped_over() {
    let mut j = vec![0xFF, 0xD8, 0xFF, 0xFF, 0xFF, 0x01, 0xFF, 0xD0];
    j.extend_from_slice(&jfif(1, 200, 200));
    assert_eq!(file_resolution(&j), Some((200.0, 200.0)));
}

#[test]
fn png_phys_in_metres_only_before_image_data() {
    assert_eq!(file_resolution(&png(&[phys(11_811, 11_811, 1)])), Some((300.0, 300.0)));
    assert_eq!(file_resolution(&png(&[phys(2, 1, 0)])), None);
    assert_eq!(file_resolution(&png(&[png_chunk(b"IDAT", &[0; 4]), phys(11_811, 11_811, 1)])), None);
    assert_eq!(file_resolution(&png(&[png_chunk(b"tEXt", b"a\0b"), phys(3_780, 3_780, 1)])).map(|p| p.0.round()), Some(96.0));
    assert_eq!(file_resolution(&png(&[])), None);
}

/// What the probe reads is what a full decode reports, for each format the encoder writes with
/// a resolution.
#[test]
fn agrees_with_decode_for_encoded_files() {
    let mut img = Image::from_raw(2, 1, ChannelLayout::Rgb, SampleType::U8, vec![10, 20, 30, 40, 50, 60]).unwrap();
    img.meta.dpi = Some((300.0, 150.0));
    for format in [Format::Jpeg, Format::Png, Format::WebP, Format::Tiff] {
        let bytes = crate::encode(&img, format, &EncodeOptions::default()).unwrap();
        let decoded = crate::decode_as(format, &bytes).unwrap().meta.dpi;
        assert_eq!(file_resolution(&bytes), decoded, "{format:?}");
        assert_eq!(decoded, Some((300.0, 150.0)), "{format:?}");
    }
}

#[test]
fn webp_without_exif_has_none() {
    let img = Image::from_raw(1, 1, ChannelLayout::Rgb, SampleType::U8, vec![1, 2, 3]).unwrap();
    let bytes = crate::encode(&img, Format::WebP, &EncodeOptions::default()).unwrap();
    assert_eq!(file_resolution(&bytes), None);
    // A RIFF/WEBP container the decoder's probe refuses gives nothing either.
    assert_eq!(file_resolution(b"RIFF\x04\0\0\0WEBP"), None);
}

#[test]
fn webp_exif_after_many_frames_and_riff_bounds() {
    let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(kind);
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
    };
    let mut body = b"WEBP".to_vec();
    chunk(&mut body, b"VP8X", &[0; 10]);
    chunk(&mut body, b"ANIM", &[0; 6]);
    for _ in 0..100 {
        chunk(&mut body, b"ANMF", &[0; 17]);
    }
    chunk(&mut body, b"EXIF", &tiff(true, 300, 300, Some(2)));
    let riff = |size: usize, trailing: usize| {
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&u32::try_from(size).unwrap().to_le_bytes());
        webp.extend_from_slice(&body);
        webp.extend_from_slice(&vec![0xAA; trailing]);
        webp
    };
    assert_eq!(file_resolution(&riff(body.len(), 0)), Some((300.0, 300.0)));
    assert_eq!(file_resolution(&riff(body.len(), 4)), Some((300.0, 300.0)));
    assert_eq!(file_resolution(&riff(body.len() - 4, 4)), None);
}

#[test]
fn bmp_pixels_per_metre() {
    close(file_resolution(&bmp(40, 3_780, 7_559)), (96.0, 192.0));
    assert_eq!(file_resolution(&bmp(12, 3_780, 3_780)), None);
    assert_eq!(file_resolution(&bmp(40, -3_780, 3_780)), None);
    assert_eq!(file_resolution(&bmp(40, 0, 0)), None);
}

#[test]
fn tiff_tags_in_both_byte_orders_and_units() {
    assert_eq!(file_resolution(&tiff(true, 300, 300, None)), Some((300.0, 300.0)));
    close(file_resolution(&tiff(false, 100, 50, Some(3))), (254.0, 127.0));
    assert_eq!(file_resolution(&tiff(true, 300, 300, Some(1))), None);
}

#[test]
fn other_formats_and_absurd_values_give_none() {
    assert_eq!(file_resolution(b"GIF89a\x01\0\x01\0"), None);
    assert_eq!(file_resolution(b""), None);
    assert_eq!(file_resolution(&tiff(true, 1_000_000, 1_000_000, None)), None);
    assert!(usable((1.0, MAX_PPI)));
    assert!(!usable((0.5, 72.0)));
    assert!(!usable((f32::NAN, 72.0)));
}

#[test]
fn truncated_and_corrupted_headers_never_panic() {
    let samples = [
        jpeg(&[jfif(1, 300, 300), exif_app1(&tiff(false, 600, 600, Some(2)))]),
        png(&[phys(11_811, 11_811, 1)]),
        bmp(40, 3_780, 3_780),
        tiff(false, 300, 300, Some(2)),
    ];
    for s in &samples {
        for cut in 0..=s.len() {
            let _ = file_resolution(&s[..cut]);
        }
        for at in 0..s.len() {
            for v in [0u8, 0x7F, 0xFF] {
                let mut c = s.clone();
                c[at] = v;
                let _ = file_resolution(&c);
            }
        }
    }
}

#[test]
fn png_phys_must_be_a_whole_nine_byte_chunk() {
    // A pHYs that declares no data doesn't borrow the bytes after it.
    let mut empty = png_chunk(b"pHYs", &[]);
    empty.extend_from_slice(&phys(11_811, 11_811, 1)[8..]);
    assert_eq!(file_resolution(&png(&[empty])), None);
    // Cut off before its CRC.
    let whole = png(&[phys(11_811, 11_811, 1)]);
    let crc_at = whole.len() - 12 - 4;
    assert_eq!(file_resolution(&whole[..crc_at]), None);
    assert_eq!(file_resolution(&whole[..crc_at + 4]), Some((300.0, 300.0)));
}

/// A PNG chunk with its real CRC, for the decoder's own probe, which checks it.
fn crc_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = png_chunk(kind, data);
    let body = c.get(4..c.len() - 4).unwrap().to_vec();
    let mut crc = !0u32;
    for b in body {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    let n = c.len();
    c[n - 4..].copy_from_slice(&(!crc).to_be_bytes());
    c
}

#[test]
fn png_zero_phys_leaves_the_exif_resolution() {
    let exif = crc_chunk(b"eXIf", &tiff(false, 600, 600, Some(2)));
    let zero = |x: u32, y: u32| {
        let mut d = x.to_be_bytes().to_vec();
        d.extend_from_slice(&y.to_be_bytes());
        d.push(1);
        crc_chunk(b"pHYs", &d)
    };
    let png = |chunks: &[Vec<u8>]| {
        let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
        p.extend_from_slice(&crc_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]));
        chunks.iter().for_each(|c| p.extend_from_slice(c));
        // The decoder's probe reads up to the image data, so there has to be some.
        p.extend_from_slice(&crc_chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0, 0, 0, 1, 0, 1]));
        p.extend_from_slice(&crc_chunk(b"IEND", &[]));
        p
    };
    assert_eq!(file_resolution(&png(std::slice::from_ref(&exif))), Some((600.0, 600.0)));
    assert_eq!(file_resolution(&png(&[zero(0, 0), exif.clone()])), Some((600.0, 600.0)));
    assert_eq!(file_resolution(&png(&[zero(11_811, 0), exif])), Some((600.0, 600.0)));
}

#[test]
fn png_pixels_per_meter_preserves_raw_values_and_shared_chunk_limit() {
    assert_eq!(crate::png_probe::pixels_per_meter(&png(&[phys(1181, 2362, 1)])), Some((1181, 2362)));
    assert_eq!(crate::png_probe::pixels_per_meter(&png(&[phys(1181, 1181, 0)])), None);
    assert_eq!(crate::png_probe::pixels_per_meter(&png(&[phys(0, 1181, 1)])), None);
    assert_eq!(crate::png_probe::pixels_per_meter(b"not a png"), None);
    assert_eq!(crate::png_probe::pixels_per_meter(&png(&[png_chunk(b"IDAT", &[]), phys(1181, 1181, 1)])), None);

    let text = png_chunk(b"tEXt", b"k\x00v");
    let mut at_limit = vec![text.clone(); super::MAX_CHUNKS - 2];
    at_limit.push(phys(1181, 1181, 1));
    assert_eq!(crate::png_probe::pixels_per_meter(&png(&at_limit)), Some((1181, 1181)));

    let mut past_limit = vec![text; super::MAX_CHUNKS - 1];
    past_limit.push(phys(1181, 1181, 1));
    assert_eq!(crate::png_probe::pixels_per_meter(&png(&past_limit)), None);
}
