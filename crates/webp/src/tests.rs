//! The animation header tests follow EffectCraft's; the writer is checked by decoding its output
//! with the `image` crate.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use image::{ExtendedColorType, ImageDecoder, ImageEncoder, codecs::webp::WebPDecoder, codecs::webp::WebPEncoder};

use super::*;

fn chunk_of(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    chunk(&mut out, fourcc, body).unwrap();
    out
}

fn u24_bytes(v: u32) -> [u8; 3] {
    [v as u8, (v >> 8) as u8, (v >> 16) as u8]
}

/// A lossless still of one colour, written by `image`.
fn lossless(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    WebPEncoder::new_lossless(&mut out).write_image(&rgba.repeat((w * h) as usize), w, h, ExtendedColorType::Rgba8).unwrap();
    out
}

/// The `VP8L` chunk of a lossless still.
fn vp8l_chunk(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let file = lossless(w, h, rgba);
    let at = file.windows(4).position(|c| c == b"VP8L").unwrap();
    file[at..].to_vec()
}

/// An animated WebP: one solid frame for each (colour, duration in ms).
fn animated(w: u32, h: u32, frames: &[([u8; 4], u32)], loops: u16) -> Vec<u8> {
    let mut body = b"WEBP".to_vec();
    let mut vp8x = vec![0x12, 0, 0, 0];
    vp8x.extend(u24_bytes(w - 1));
    vp8x.extend(u24_bytes(h - 1));
    body.extend(chunk_of(b"VP8X", &vp8x));
    body.extend(chunk_of(b"ANIM", &[0, 0, 0, 0, loops as u8, (loops >> 8) as u8]));
    for (rgba, ms) in frames {
        let mut anmf = Vec::new();
        anmf.extend([0; 6]);
        anmf.extend(u24_bytes(w - 1));
        anmf.extend(u24_bytes(h - 1));
        anmf.extend(u24_bytes(*ms));
        anmf.push(0x02);
        anmf.extend(vp8l_chunk(w, h, *rgba));
        body.extend(chunk_of(b"ANMF", &anmf));
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const HALF: [u8; 4] = [0, 0, 255, 128];

#[test]
fn an_animation_reports_canvas_frames_and_loop_count() {
    let bytes = animated(4, 2, &[(RED, 50), (GREEN, 100), (HALF, 50)], 3);
    assert!(is_webp(&bytes));
    let info = scan(&bytes, 100).unwrap();
    assert_eq!((info.width, info.height, info.alpha, info.animated, info.loop_count, info.complete), (4, 2, true, true, Some(3), true));
    assert!(info.is_animation());
    assert_eq!(info.frames.iter().map(|f| f.duration_ms).collect::<Vec<_>>(), [50, 100, 50]);
    assert!(info.frames.iter().all(|f| (f.x, f.y, f.width, f.height, f.blend, f.dispose_to_background) == (0, 0, 4, 2, false, false)));
    // The `image` crate's decoder agrees it is an animation of this canvas.
    let d = WebPDecoder::new(std::io::Cursor::new(&bytes)).unwrap();
    assert!(d.has_animation());
    assert_eq!(d.dimensions(), (4, 2));
}

#[test]
fn frame_fields_are_decoded() {
    let mut bytes = animated(8, 8, &[(RED, 0), (GREEN, 0xABCDE)], 0);
    let at = bytes.windows(4).position(|c| c == b"ANMF").unwrap() + 8;
    bytes[at..at + 3].copy_from_slice(&u24_bytes(1));
    bytes[at + 3..at + 6].copy_from_slice(&u24_bytes(2));
    bytes[at + 15] = 0x01;
    let info = scan(&bytes, 100).unwrap();
    let f = info.frames[0];
    assert_eq!((f.x, f.y, f.blend, f.dispose_to_background, f.duration_ms), (2, 4, true, true, 0));
    assert_eq!((info.frames[1].duration_ms, info.loop_count), (0xABCDE, Some(0)));
}

#[test]
fn stills_report_their_size_and_are_not_animations() {
    // Lossless simple form: the size and alpha bit come from the VP8L header.
    let info = scan(&lossless(5, 3, HALF), 100).unwrap();
    assert_eq!((info.width, info.height, info.alpha, info.animated, info.is_animation(), info.complete), (5, 3, true, false, false, true));
    assert_eq!(scan(&lossless(300, 200, RED), 100).map(|i| (i.width, i.height)), Some((300, 200)));
    // Lossy simple form: the size comes from the VP8 key frame header.
    let frame = [0x10, 0x02, 0x00, 0x9D, 0x01, 0x2A, 0x40, 0x01, 0xF0, 0x00];
    let file = still(320, 240, &frame, &Extras::default()).unwrap();
    let info = scan(&file, 100).unwrap();
    assert_eq!((info.width, info.height, info.alpha, info.animated), (320, 240, false, false));
    // The animation flag with a single frame is still shown as a still.
    assert!(!scan(&animated(2, 2, &[(RED, 10)], 0), 100).unwrap().is_animation());
    assert_eq!(scan(b"RIFF\x04\x00\x00\x00WEBP", 100), None);
    assert_eq!(scan(b"\x89PNG\r\n\x1a\n....", 100), None);
    assert!(!is_webp(b"RIFF....WAVE") && !is_webp(b"RIFF"));
}

#[test]
fn the_frame_limit_stops_the_walk() {
    let bytes = animated(2, 2, &[(RED, 10), (GREEN, 20), (RED, 30)], 0);
    let info = scan(&bytes, 2).unwrap();
    assert_eq!((info.frames.len(), info.complete), (2, false));
    let info = scan(&bytes, 3).unwrap();
    assert_eq!((info.frames.len(), info.complete), (3, true));
    assert_eq!(scan(&bytes, 0).unwrap().frames.len(), 0);
}

#[test]
fn a_cut_or_damaged_file_gives_what_was_read() {
    let bytes = animated(4, 2, &[(RED, 50), (GREEN, 100), (HALF, 50)], 0);
    let whole = scan(&bytes, 100).unwrap();
    for cut in 0..bytes.len() {
        if let Some(info) = scan(&bytes[..cut], 100) {
            assert!(!info.complete && info.frames.len() <= whole.frames.len(), "cut {cut}");
            assert_eq!((info.width, info.height), (4, 2));
        }
    }
    for i in 0..bytes.len() {
        for v in [0u8, 1, 0x7F, 0xFF] {
            let mut b = bytes.clone();
            b[i] = v;
            if let Some(info) = scan(&b, 100) {
                assert!(info.frames.len() <= 100);
            }
        }
    }
    // A cut inside the last frame's picture keeps that frame's header.
    assert_eq!(scan(&bytes[..bytes.len() - 3], 100).map(|i| (i.frames.len(), i.complete)), Some((3, false)));
    // A chunk size running past the file ends the walk.
    let mut far = bytes.clone();
    let at = far.windows(4).position(|c| c == b"ANIM").unwrap();
    far[at + 4..at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(scan(&far, 100).map(|i| (i.frames.len(), i.complete)), Some((0, false)));
}

/// A stand-in for an encoded VP8 frame: a real key frame header for 16 × 16 (frame tag, start
/// code, sizes) followed by filler. The container tests read headers only and never decode it.
const VP8_FRAME: &[u8] =
    &[0x30, 0x01, 0x00, 0x9D, 0x01, 0x2A, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00, 0x1F, 0x25, 0xA4, 0x00, 0x03, 0x70, 0x00, 0xFE, 0xFB, 0x94, 0x00, 0x00];

#[test]
fn the_simple_form_is_one_chunk() {
    let file = still(16, 16, VP8_FRAME, &Extras::default()).unwrap();
    assert_eq!(&file[..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize, file.len() - 8);
    assert_eq!(&file[8..16], b"WEBPVP8 ");
    assert_eq!(&file[20..], VP8_FRAME);
    // An odd payload is padded to an even length, and the size field keeps the real length.
    let odd = still(16, 16, &VP8_FRAME[..23], &Extras::default()).unwrap();
    assert_eq!((odd.len(), u32::from_le_bytes(odd[16..20].try_into().unwrap())), (20 + 24, 23));
}

#[test]
fn the_extended_form_orders_its_chunks() {
    let alpha = vec![128u8; 256];
    let extras = Extras { alpha: Some(&alpha), icc: Some(b"icc"), exif: Some(b"exif!"), xmp: Some(b"<x/>") };
    let file = still(16, 16, VP8_FRAME, &extras).unwrap();
    let mut at = 12;
    let mut seen = Vec::new();
    while at < file.len() {
        let size = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
        seen.push((String::from_utf8_lossy(&file[at..at + 4]).into_owned(), size));
        at += 8 + size + (size & 1);
    }
    assert_eq!(at, file.len());
    assert_eq!(
        seen,
        [("VP8X".into(), 10), ("ICCP".into(), 3), ("ALPH".into(), 257), ("VP8 ".into(), VP8_FRAME.len()), ("EXIF".into(), 5), ("XMP ".into(), 4)]
    );
    // Flags: ICC, alpha, EXIF, XMP; canvas 16 × 16 stored minus one.
    assert_eq!(&file[20..30], &[0x3C, 0, 0, 0, 15, 0, 0, 15, 0, 0]);
    let info = scan(&file, 10).unwrap();
    assert_eq!((info.width, info.height, info.alpha, info.animated, info.complete), (16, 16, true, false, true));
    // Only the flags of what is present are set.
    let file = still(16, 16, VP8_FRAME, &Extras { xmp: Some(b"<x/>"), ..Extras::default() }).unwrap();
    assert_eq!(file[20], 0x04);
    // The `image` crate reads the container: size, alpha and the profile.
    let file = still(16, 16, VP8_FRAME, &Extras { alpha: Some(&alpha), icc: Some(b"icc"), ..Extras::default() }).unwrap();
    let mut d = WebPDecoder::new(std::io::Cursor::new(&file)).unwrap();
    assert_eq!((d.dimensions(), d.color_type().has_alpha()), ((16, 16), true));
    assert_eq!(d.icc_profile().unwrap().as_deref(), Some(&b"icc"[..]));
}

#[test]
fn the_writer_refuses_what_does_not_fit() {
    let e = Extras::default();
    assert!(still(0, 16, VP8_FRAME, &e).is_err());
    assert!(still(16, 0, VP8_FRAME, &e).is_err());
    assert!(still(MAX_VP8_DIMENSION + 1, 16, VP8_FRAME, &e).is_err());
    assert!(still(MAX_VP8_DIMENSION, MAX_VP8_DIMENSION, VP8_FRAME, &e).is_ok());
    assert_eq!(
        still(16, 16, VP8_FRAME, &Extras { alpha: Some(&[0; 255]), ..e }),
        Err(Error::Encode("the alpha plane does not match the picture size"))
    );
}
