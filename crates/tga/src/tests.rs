//! The reader's tests come from FilmCraft's decoder, the writer's from PhotoCraft's encoder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn header(kind: u8, w: u16, h: u16, depth: u8, descriptor: u8) -> Vec<u8> {
    let mut b = vec![0, 0, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    b.extend_from_slice(&[depth, descriptor]);
    b
}

fn dec(bytes: &[u8]) -> Result<Picture> {
    decode(bytes, &Limits::default())
}

#[test]
fn bottom_up_bgr_is_flipped() {
    let mut f = header(2, 2, 2, 24, 0);
    // Bottom row first: blue, green; then the top row: red, white.
    f.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
    let r = dec(&f).unwrap();
    assert_eq!((r.width, r.height, r.has_alpha, r.layout), (2, 2, false, Layout::Rgb));
    assert_eq!(r.rgba, [255, 0, 0, 255, 255, 255, 255, 255, 0, 0, 255, 255, 0, 255, 0, 255]);
}

#[test]
fn every_origin_corner() {
    // 2 × 2 gray, stored values 1 2 / 3 4 in file order.
    for (descriptor, want) in [(0x00, [3, 4, 1, 2]), (0x10, [4, 3, 2, 1]), (0x20, [1, 2, 3, 4]), (0x30, [2, 1, 4, 3])] {
        let mut f = header(3, 2, 2, 8, descriptor);
        f.extend_from_slice(&[1, 2, 3, 4]);
        let r = dec(&f).unwrap();
        assert_eq!(r.rgba.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), want, "descriptor {descriptor:#x}");
        assert_eq!(r.layout, Layout::Gray);
    }
}

#[test]
fn rle_runs_and_raw_packets_cross_lines() {
    let mut f = header(10, 3, 2, 32, 0x28);
    // A run of 4 translucent red, then 2 raw pixels.
    f.extend_from_slice(&[0x83, 0, 0, 255, 128, 0x01, 1, 2, 3, 255, 4, 5, 6, 255]);
    let r = dec(&f).unwrap();
    assert!(r.has_alpha);
    assert_eq!(r.layout, Layout::Rgba);
    assert_eq!(&r.rgba[..16], &[255, 0, 0, 128].repeat(4)[..]);
    assert_eq!(&r.rgba[16..], &[3, 2, 1, 255, 6, 5, 4, 255]);
}

#[test]
fn colour_map_and_16_bit_alpha() {
    let mut f = header(1, 2, 1, 8, 0x20);
    f[1] = 1; // colour map present
    f[3..5].copy_from_slice(&1u16.to_le_bytes()); // first entry index 1
    f[5..7].copy_from_slice(&2u16.to_le_bytes());
    f[7] = 24;
    f.extend_from_slice(&[10, 20, 30, 40, 50, 60, 2, 1]);
    let r = dec(&f).unwrap();
    assert_eq!(r.rgba, [60, 50, 40, 255, 30, 20, 10, 255]);
    assert_eq!(r.layout, Layout::Rgb);
    f[18 + 6] = 0; // index 0 is below the map's first entry
    assert_eq!(dec(&f), Err(Error::Malformed("colour index outside the colour map")));

    let mut f = header(2, 2, 1, 16, 0x21);
    f.extend_from_slice(&0xFC00u16.to_le_bytes()); // opaque full red
    f.extend_from_slice(&0x03E0u16.to_le_bytes()); // transparent full green
    let r = dec(&f).unwrap();
    assert_eq!(r.rgba, [255, 0, 0, 255, 0, 255, 0, 0]);

    // 16-bit grayscale with alpha.
    let mut f = header(3, 1, 1, 16, 0x28);
    f.extend_from_slice(&[7, 9]);
    let r = dec(&f).unwrap();
    assert_eq!((r.rgba.as_slice(), r.layout, r.has_alpha), (&[7, 7, 7, 9][..], Layout::GrayAlpha, true));
}

#[test]
fn zero_alpha_32_bit_without_attribute_bits_is_opaque() {
    let mut f = header(2, 1, 1, 32, 0);
    f.extend_from_slice(&[1, 2, 3, 0]);
    let r = dec(&f).unwrap();
    assert_eq!((r.rgba.as_slice(), r.has_alpha), (&[3, 2, 1, 255][..], false));
}

#[test]
fn hostile_headers_fail_without_allocating() {
    // 65535 × 65535 run-length encoded from a few bytes, truncated raw data, bad types.
    let mut f = header(10, 65535, 65535, 32, 0);
    f.extend_from_slice(&[0xFF, 1, 2, 3, 4]);
    assert_eq!(dec(&f), Err(Error::LimitExceeded("pixel count")));
    let mut f = header(10, 16384, 16384, 32, 0);
    f.extend_from_slice(&[0xFF, 1, 2, 3, 4]);
    assert_eq!(dec(&f), Err(Error::Malformed("truncated run-length data")));
    let mut f = header(2, 4, 4, 24, 0);
    f.extend_from_slice(&[0; 10]);
    assert_eq!(dec(&f), Err(Error::Malformed("truncated pixel data")));
    assert!(dec(&header(2, 1, 1, 7, 0)).is_err());
    assert!(dec(&header(1, 1, 1, 8, 0)).is_err());
    assert_eq!(dec(&header(2, 0, 1, 24, 0)), Err(Error::Malformed("empty picture")));
    assert_eq!(dec(&header(4, 1, 1, 24, 0)), Err(Error::Unsupported(4)));
    assert_eq!(dec(&[0; 5]), Err(Error::Malformed("truncated header")));
    for n in 0..40 {
        let mut f = header(9, 3, 3, 8, 0);
        f[1] = 1;
        f[5] = 2;
        f[7] = 16;
        f.extend_from_slice(&[0x82, 1, 0, 1, 0x01, 0, 0]);
        f.truncate(n);
        let _ = dec(&f);
    }
}

#[test]
fn the_pixel_limit_is_checked() {
    let mut f = header(3, 2, 2, 8, 0);
    f.extend_from_slice(&[0; 4]);
    assert!(decode(&f, &Limits { max_pixels: 4 }).is_ok());
    assert_eq!(decode(&f, &Limits { max_pixels: 3 }), Err(Error::LimitExceeded("pixel count")));
}

#[test]
fn sniffs_by_footer_or_name() {
    let mut f = header(2, 1, 1, 24, 0);
    f.extend_from_slice(&[0, 0, 0]);
    assert!(sniff("x.TGA", &f));
    assert!(!sniff("x.bin", &f));
    f.extend_from_slice(&[0; 8]);
    f.extend_from_slice(SIGNATURE);
    assert!(sniff("x.bin", &f));
    assert!(!sniff("x.tga", b"\x89PNG\r\n\x1a\n0000000000"));
}

#[test]
fn run_packets_at_the_128_pixel_boundary() {
    for bpp in 1..=4 {
        for (pixels, headers) in [(1, vec![0]), (2, vec![0x81]), (127, vec![0xfe]), (128, vec![0xff]), (129, vec![0xff, 0])] {
            let pixel: Vec<u8> = (1..=bpp as u8).collect();
            let mut encoded = Vec::new();
            rle_row(&pixel.repeat(pixels), bpp, &mut encoded);
            let expected: Vec<u8> = headers.into_iter().flat_map(|header| std::iter::once(header).chain(pixel.iter().copied())).collect();
            assert_eq!(encoded, expected, "{pixels} pixels, {bpp} bytes per pixel");
        }
    }
}

#[test]
fn raw_packets_at_the_128_pixel_boundary() {
    for bpp in 1..=4 {
        for pixels in [127usize, 128, 129] {
            let row: Vec<u8> = (0..pixels).flat_map(|x| vec![x as u8; bpp]).collect();
            let mut encoded = Vec::new();
            rle_row(&row, bpp, &mut encoded);
            let mut expected = vec![(pixels.min(128) - 1) as u8];
            expected.extend_from_slice(&row[..pixels.min(128) * bpp]);
            if pixels > 128 {
                expected.push(0);
                expected.extend_from_slice(&row[128 * bpp..]);
            }
            assert_eq!(encoded, expected, "{pixels} pixels, {bpp} bytes per pixel");
        }
    }
}

#[test]
fn raw_packets_stop_before_a_run_of_complete_pixels() {
    // Equal first channels alone do not make an RGBA run; alpha participates too.
    let a = [10, 20, 30, 40];
    let b = [10, 20, 30, 41];
    let c = [50, 60, 70, 80];
    let row = [a, b, c, c, a].concat();
    let mut encoded = Vec::new();
    rle_row(&row, 4, &mut encoded);
    assert_eq!(encoded, [vec![1], a.to_vec(), b.to_vec(), vec![0x81], c.to_vec(), vec![0], a.to_vec()].concat());
}

#[test]
fn dimensions_fit_the_unsigned_16_bit_header() {
    for (width, height) in [(65535u32, 1u32), (1, 65535)] {
        let file = encode(width, height, Layout::Gray, &vec![0; 65535]).unwrap();
        assert_eq!(&file[12..14], &(width as u16).to_le_bytes());
        assert_eq!(&file[14..16], &(height as u16).to_le_bytes());
    }
    for (width, height) in [(65536u32, 1u32), (1, 65536)] {
        assert!(matches!(encode(width, height, Layout::Gray, &vec![0; 65536]), Err(Error::Encode(_))), "{width}x{height}");
    }
}

#[test]
fn wrong_buffer_sizes_are_refused() {
    for layout in [Layout::Gray, Layout::GrayAlpha, Layout::Rgb, Layout::Rgba] {
        let len = 2 * 2 * layout.channels();
        for bytes in [len - 1, len + 1] {
            assert!(matches!(encode(2, 2, layout, &vec![0; bytes]), Err(Error::Encode(_))), "{layout:?}: {bytes} bytes");
        }
    }
    assert!(encode(0, 0, Layout::Rgb, &[]).is_ok());
}

/// What the writer produces, the reader returns: every layout, with rows that hold runs, raw
/// stretches and packets longer than 128 pixels, and no packet crossing a row.
#[test]
fn written_files_read_back() {
    for layout in [Layout::Gray, Layout::GrayAlpha, Layout::Rgb, Layout::Rgba] {
        let n = layout.channels();
        let (w, h) = (300usize, 3usize);
        let data: Vec<u8> =
            (0..w * h).flat_map(|i| (0..n).map(move |c| if i % w < 140 { 9 + c as u8 } else { (i * 7 + c * 31) as u8 | 1 })).collect();
        let file = encode(w as u32, h as u32, layout, &data).unwrap();
        assert!(file.ends_with(SIGNATURE) && sniff("x.bin", &file));
        // Walk the packets: each row's packets add up to exactly the row.
        let mut at = HEADER;
        for _ in 0..h {
            let mut px = 0;
            while px < w {
                let count = usize::from(file[at] & 0x7F) + 1;
                at += 1 + if file[at] & 0x80 != 0 { n } else { count * n };
                px += count;
            }
            assert_eq!(px, w, "a packet crossed a row");
        }
        assert_eq!(at, file.len() - FOOTER.len());
        let r = dec(&file).unwrap();
        assert_eq!((r.width, r.height, r.layout), (w as u32, h as u32, layout));
        let want: Vec<u8> = data
            .chunks(n)
            .flat_map(|p| match layout {
                Layout::Gray => [p[0], p[0], p[0], 255],
                Layout::GrayAlpha => [p[0], p[0], p[0], p[1]],
                Layout::Rgb => [p[0], p[1], p[2], 255],
                Layout::Rgba => [p[0], p[1], p[2], p[3]],
            })
            .collect();
        assert_eq!(r.rgba, want, "{layout:?}");
    }
}
