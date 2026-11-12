//! The palette tests are PhotoCraft's; the scan is also checked against files written by the
//! `image` crate's GIF encoder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

/// A GIF up to its first image: `global` table entries, optional GCE transparency, optional
/// local table.
fn gif(global: &[[u8; 3]], transparent: Option<u8>, local: Option<&[[u8; 3]]>) -> Vec<u8> {
    let size_bits = |n: usize| (n.max(2).next_power_of_two().trailing_zeros() - 1) as u8;
    let mut b = b"GIF89a".to_vec();
    b.extend([4, 0, 4, 0]);
    b.push(if global.is_empty() { 0 } else { 0x80 | size_bits(global.len()) });
    b.extend([0, 0]);
    let pad = |b: &mut Vec<u8>, t: &[[u8; 3]]| {
        let n = 2usize << size_bits(t.len());
        for k in 0..n {
            b.extend(t.get(k).copied().unwrap_or([0, 0, 0]));
        }
    };
    if !global.is_empty() {
        pad(&mut b, global);
    }
    // A comment extension first, to skip.
    b.extend([0x21, 0xFE, 3, b'h', b'i', b'!', 0]);
    if let Some(i) = transparent {
        b.extend([0x21, 0xF9, 4, 0x01, 0, 0, i, 0]);
    }
    b.extend([0x2C, 0, 0, 0, 0, 4, 0, 4, 0]);
    match local {
        Some(t) => {
            b.push(0x80 | size_bits(t.len()));
            pad(&mut b, t);
        }
        None => b.push(0),
    }
    b.extend([2, 2, 0x4C, 0x01, 0, 0x3B]);
    b
}

/// An animated GIF of solid 2 × 2 frames with these delays (hundredths), written by `image`.
fn animated(delays_cs: &[u32], repeat: Option<image::codecs::gif::Repeat>) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        if let Some(r) = repeat {
            enc.set_repeat(r).unwrap();
        }
        for (i, &cs) in delays_cs.iter().enumerate() {
            let buf = image::RgbaImage::from_pixel(2, 2, image::Rgba([i as u8 * 40, 0, 0, 255]));
            enc.encode_frame(image::Frame::from_parts(buf, 0, 0, image::Delay::from_numer_denom_ms(cs * 10, 1))).unwrap();
        }
    }
    out
}

#[test]
fn the_first_frames_table_and_transparent_entry() {
    let g = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
    assert_eq!(palette(&gif(&g, None, None)), Some((g.to_vec(), None)));
    assert_eq!(palette(&gif(&g, Some(2), None)), Some((g.to_vec(), Some(2))));
    // A local table wins over the global one.
    let l = [[1, 2, 3], [4, 5, 6]];
    assert_eq!(palette(&gif(&g, Some(1), Some(&l))), Some((l.to_vec(), Some(1))));
    // A transparent index past the table is no transparent entry.
    assert_eq!(palette(&gif(&l, Some(9), None)), Some((l.to_vec(), None)));
}

#[test]
fn no_table_or_not_a_gif_is_none() {
    assert_eq!(palette(&gif(&[], None, None)), None, "no table at all");
    assert_eq!(palette(b"\x89PNG\r\n\x1a\n"), None);
    assert_eq!(palette(b""), None);
    assert_eq!(scan(b"\x89PNG\r\n\x1a\n", 10), None);
    assert_eq!(scan(b"GIF89a\x01\x00", 10), None);
    assert!(sniff(b"GIF87a") && sniff(b"GIF89a...") && !sniff(b"GIF88a"));
}

#[test]
fn a_scan_reports_the_screen_and_the_frame() {
    let g = [[9, 9, 9], [8, 8, 8]];
    let info = scan(&gif(&g, Some(1), None), 10).unwrap();
    assert_eq!((info.width, info.height, info.global_table, info.complete, info.loop_count), (4, 4, true, true, None));
    let [f] = info.frames.as_slice() else { panic!("{:?}", info.frames) };
    assert_eq!((f.left, f.top, f.width, f.height, f.delay_cs, f.transparent, f.disposal, f.local_table), (0, 0, 4, 4, 0, Some(1), 0, false));
    assert_eq!(info.has_more_frames(), Some(false));
    let info = scan(&gif(&g, None, Some(&g)), 10).unwrap();
    assert_eq!((info.frames[0].local_table, info.frames[0].transparent), (true, None));
}

#[test]
fn an_animation_reports_every_delay_and_the_loop_count() {
    use image::codecs::gif::Repeat;
    let bytes = animated(&[5, 10, 0, 7], Some(Repeat::Infinite));
    let info = scan(&bytes, 100).unwrap();
    assert_eq!((info.width, info.height, info.complete, info.loop_count), (2, 2, true, Some(0)));
    assert_eq!(info.frames.iter().map(|f| f.delay_cs).collect::<Vec<_>>(), [5, 10, 0, 7]);
    assert_eq!((info.total_delay_cs(), info.has_more_frames()), (22, Some(true)));
    assert!(info.frames.iter().all(|f| (f.width, f.height) == (2, 2)));
    assert_eq!(scan(&animated(&[5, 5], Some(Repeat::Finite(3))), 100).unwrap().loop_count, Some(3));
    assert_eq!(scan(&animated(&[5], None), 100).unwrap().has_more_frames(), Some(false));
}

#[test]
fn the_frame_limit_stops_the_walk() {
    let bytes = animated(&[1, 2, 3, 4, 5], None);
    let info = scan(&bytes, 3).unwrap();
    assert_eq!((info.frames.len(), info.complete, info.has_more_frames()), (3, false, Some(true)));
    let info = scan(&bytes, 5).unwrap();
    assert_eq!((info.frames.len(), info.complete), (5, true));
    // One frame recorded and the walk stopped: whether more exist is not known.
    let info = scan(&bytes, 1).unwrap();
    assert_eq!((info.frames.len(), info.has_more_frames()), (1, None));
    assert_eq!(scan(&bytes, 0).unwrap().frames.len(), 0);
}

#[test]
fn a_cut_or_damaged_file_gives_what_was_read() {
    let bytes = animated(&[4, 4, 4], None);
    let whole = scan(&bytes, 100).unwrap();
    for cut in 0..bytes.len() {
        // Never a panic; never complete; never more frames than the whole file has.
        if let Some(info) = scan(&bytes[..cut], 100) {
            assert!(!info.complete && info.frames.len() <= whole.frames.len(), "cut {cut}");
        }
        let _ = palette(&bytes[..cut]);
    }
    for i in 0..bytes.len() {
        for v in [0u8, 0x21, 0x2C, 0x3B, 0xFF] {
            let mut b = bytes.clone();
            b[i] = v;
            if let Some(info) = scan(&b, 100) {
                assert!(info.frames.iter().all(|f| f.offset < b.len()));
            }
            let _ = palette(&b);
        }
    }
    // A cut inside the second frame keeps the first.
    let second = whole.frames[1].offset;
    let info = scan(&bytes[..second + 5], 100).unwrap();
    assert_eq!((info.frames.len(), info.complete, info.has_more_frames()), (1, false, None));
}

#[test]
fn plausibility_of_a_screen_for_a_file_size() {
    // A 20-byte file declaring a 65535 × 65535 screen around a 1 × 1 frame.
    let mut b = gif(&[[0, 0, 0], [1, 1, 1]], None, None);
    b[6..10].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let n = b.len();
    b[n - 11..n - 7].copy_from_slice(&[1, 0, 1, 0]);
    let info = scan(&b, 1).unwrap();
    assert_eq!((info.width, info.frames[0].width, info.frames[0].height), (65535, 1, 1));
    let screen = 65535u64 * 65535 * 4;
    // 256 MiB of screen is free; the rest must be backed by file.
    assert_eq!(info.bytes_to_back(4, 1 << 28), screen - (1 << 28));
    assert!(!plausible(info.bytes_to_back(4, 1 << 28), b.len(), 1 << 14));
    // A small screen needs only its first frame backed.
    let small = scan(&gif(&[[0, 0, 0], [1, 1, 1]], None, None), 1).unwrap();
    assert_eq!(small.bytes_to_back(4, 1 << 28), 64);
    assert!(plausible(64, 40, 1 << 14));
    // The ratio and the 1 MiB allowance, at and past the boundary.
    assert!(plausible((1 << 20) + 10 * 4, 10, 4));
    assert!(!plausible((1 << 20) + 10 * 4 + 1, 10, 4));
}
