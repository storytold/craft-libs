//! Ported from FilmCraft's Radiance HDR reader tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn file(res: &str, body: &[u8]) -> Vec<u8> {
    let mut v = format!("#?RADIANCE\n# test\nFORMAT=32-bit_rle_rgbe\nEXPOSURE=1.0\n\n{res}\n").into_bytes();
    v.extend_from_slice(body);
    v
}

fn dec(b: &[u8]) -> Result<Picture> {
    decode(b, &Limits::default())
}

/// New-style RLE of one scan line, as Radiance's own writer does: per channel, runs of 3+ equal bytes.
fn rle_line(line: &[[u8; 4]]) -> Vec<u8> {
    let n = line.len();
    let mut out = vec![2, 2, (n >> 8) as u8, n as u8];
    for c in 0..4 {
        let ch: Vec<u8> = line.iter().map(|p| p[c]).collect();
        let mut i = 0;
        while i < n {
            let run = ch[i..].iter().take(127).take_while(|&&b| b == ch[i]).count();
            if run >= 3 {
                out.extend([128 + run as u8, ch[i]]);
                i += run;
            } else {
                let mut j = i + 1;
                while j < n && j - i < 128 && !(j + 2 < n && ch[j] == ch[j + 1] && ch[j] == ch[j + 2]) {
                    j += 1;
                }
                out.push((j - i) as u8);
                out.extend_from_slice(&ch[i..j]);
                i = j;
            }
        }
    }
    out
}

#[test]
fn rgbe_values() {
    assert_eq!(rgbe_to_linear([0, 0, 0, 0]), [0.0; 3]);
    assert_eq!(rgbe_to_linear([128, 128, 128, 129]), [1.0; 3]);
    assert_eq!(rgbe_to_linear([128, 64, 0, 136]), [128.0, 64.0, 0.0]);
    assert_eq!(rgbe_to_linear([128, 128, 128, 128])[0], 0.5);
    assert!(rgbe_to_linear([255, 255, 255, 255])[0].is_finite());
}

#[test]
fn flat_top_to_bottom() {
    let body = [[10u8, 20, 30, 136], [1, 2, 3, 136], [4, 5, 6, 137], [7, 8, 9, 135]].concat();
    let f = dec(&file("-Y 2 +X 2", &body)).unwrap();
    assert_eq!((f.width, f.height), (2, 2));
    // Exponent 137 scales by 2, 135 by 1/2.
    assert_eq!(f.rgb, [10.0, 20.0, 30.0, 1.0, 2.0, 3.0, 8.0, 10.0, 12.0, 3.5, 4.0, 4.5]);
    // The `#?RGBE` signature and CRLF header lines read the same.
    let mut crlf = b"#?RGBE\r\nFORMAT=32-bit_rle_rgbe\r\n\r\n-Y 2 +X 2\r\n".to_vec();
    crlf.extend_from_slice(&body);
    assert_eq!(dec(&crlf).unwrap(), f);
}

#[test]
fn orientations() {
    // 3 wide, 2 high, pixel value = its index in the file.
    let body: Vec<u8> = (0..6u8).flat_map(|i| [i, 0, 0, 136]).collect();
    let red = |res: &str| -> Vec<f32> { dec(&file(res, &body)).unwrap().rgb.chunks(3).map(|p| p[0]).collect() };
    assert_eq!(red("-Y 2 +X 3"), [0., 1., 2., 3., 4., 5.]);
    assert_eq!(red("+Y 2 +X 3"), [3., 4., 5., 0., 1., 2.]);
    assert_eq!(red("-Y 2 -X 3"), [2., 1., 0., 5., 4., 3.]);
    // Scan lines are columns: file index = x * 2 + (row, top first for -Y).
    assert_eq!(red("+X 3 -Y 2"), [0., 2., 4., 1., 3., 5.]);
    assert_eq!(red("-X 3 +Y 2"), [5., 3., 1., 4., 2., 0.]);
    let f = dec(&file("+X 3 -Y 2", &body)).unwrap();
    assert_eq!((f.width, f.height), (3, 2));
}

#[test]
fn run_length_lines() {
    let w = 40usize;
    let rows: Vec<Vec<[u8; 4]>> =
        (0..3).map(|y| (0..w).map(|x| if x < 20 { [100, 50, 25, 130] } else { [(x * 5 + y) as u8, 7, 9, 131] }).collect()).collect();
    let body: Vec<u8> = rows.iter().flat_map(|r| rle_line(r)).collect();
    let f = dec(&file(&format!("-Y 3 +X {w}"), &body)).unwrap();
    for (y, r) in rows.iter().enumerate() {
        for (x, q) in r.iter().enumerate() {
            assert_eq!(&f.rgb[(y * w + x) * 3..(y * w + x) * 3 + 3], &rgbe_to_linear(*q), "pixel {x},{y}");
        }
    }
}

#[test]
fn old_style_runs() {
    // One pixel, then (1,1,1,3) repeats it 3 times, then a new pixel.
    let body = [[5u8, 6, 7, 136], [1, 1, 1, 3], [9, 9, 9, 136]].concat();
    let f = dec(&file("-Y 1 +X 5", &body)).unwrap();
    assert_eq!(f.rgb.chunks(3).map(|p| p[0]).collect::<Vec<_>>(), [5.0, 5.0, 5.0, 5.0, 9.0]);
}

#[test]
fn hostile_input_is_an_error_not_a_panic() {
    let ok = file("-Y 2 +X 2", &[0; 16]);
    assert!(dec(&ok).is_ok());
    for cut in 0..ok.len() - 1 {
        // A cut anywhere is an error (the header or the pixel data is incomplete), never a panic.
        assert!(dec(&ok[..cut]).is_err(), "{cut}");
    }
    assert_eq!(dec(b"P6 1 1 255 abc"), Err(Error::NotHdr));
    assert_eq!(dec(&file("-Y 2 +X 2", &[0; 8])), Err(Error::Malformed("truncated pixel data")));
    assert_eq!(dec(&file("-Y 0 +X 4", &[])), Err(Error::Malformed("empty picture")));
    assert_eq!(dec(&file("-Y 2 -Y 2", &[0; 16])), Err(Error::Malformed("resolution line names one axis twice")));
    assert_eq!(dec(&file("junk", &[0; 16])), Err(Error::Malformed("bad resolution line")));
    assert!(matches!(dec(b"#?RADIANCE\nFORMAT=32-bit_rle_xyze\n\n-Y 1 +X 1\n\0\0\0\0"), Err(Error::Unsupported(_))));
    assert!(matches!(dec(b"#?RADIANCE\nFORMAT=other\n\n-Y 1 +X 1\n\0\0\0\0"), Err(Error::Unsupported(_))));
    // An RLE line announcing the wrong width, a run past the end, a run at the start of a flat line.
    assert_eq!(dec(&file("-Y 1 +X 10", &[2, 2, 0, 11, 0])), Err(Error::Malformed("scan line length does not match the picture")));
    assert_eq!(dec(&file("-Y 1 +X 10", &[2, 2, 0, 10, 200, 1])), Err(Error::Malformed("run past the end of the scan line")));
    assert_eq!(dec(&file("-Y 1 +X 10", &[2, 2, 0, 10, 0])), Err(Error::Malformed("empty literal run")));
    assert_eq!(dec(&file("-Y 1 +X 3", &[1, 1, 1, 9, 0, 0, 0, 0])), Err(Error::Malformed("run with no previous pixel")));
    assert!(dec(b"#?RADIANCE").is_err());
}

#[test]
fn limits_are_checked_before_allocating() {
    assert_eq!(dec(&file("-Y 99999999 +X 99999999", &[0; 16])), Err(Error::LimitExceeded("pixel count")));
    assert_eq!(dec(&file("-Y 99999999999 +X 1", &[0; 16])), Err(Error::LimitExceeded("pixel count")));
    // Within the pixel limit and past the coarse size check, but the scan lines are not there:
    // refused by validation, before the float picture is allocated.
    assert_eq!(dec(&file("-Y 67108864 +X 1", &[0; 16_384])), Err(Error::Malformed("truncated pixel data")));
    let ok = file("-Y 2 +X 2", &[0; 16]);
    assert!(decode(&ok, &Limits { max_pixels: 4, ..Limits::default() }).is_ok());
    assert_eq!(decode(&ok, &Limits { max_pixels: 3, ..Limits::default() }), Err(Error::LimitExceeded("pixel count")));
    // A header without its blank line inside the header limit.
    let mut long = b"#?RADIANCE\n".to_vec();
    for _ in 0..8 {
        long.extend_from_slice(b"# 0123456789\n");
    }
    long.extend_from_slice(b"\n-Y 1 +X 1\n\0\0\0\0");
    assert!(dec(&long).is_ok());
    assert_eq!(decode(&long, &Limits { max_header: 64, ..Limits::default() }), Err(Error::LimitExceeded("header length")));
    assert_eq!(decode(&long, &Limits { max_header: 8, ..Limits::default() }), Err(Error::Malformed("truncated header")));
    assert_eq!((Limits::default().max_pixels, Limits::default().max_header), (1 << 26, 64 << 10));
}
