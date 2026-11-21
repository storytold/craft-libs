//! Uncompressed packed pictures: packing and unpacking against each other, against ffmpeg as the
//! reference (oracle; skipped when ffmpeg is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`),
//! and on bad input.

use craft_video::rawvideo::{Error, Frame, Kind, Layout, decode, encode};
use std::path::Path;
use std::process::Command;

const LAYOUTS: [Layout; 6] = [Layout::Uyvy, Layout::Yuyv, Layout::Yvyu, Layout::V210, Layout::R210, Layout::R10k];

/// A frame of pseudo-random samples at the layout's depth.
fn random_frame(layout: Layout, w: u32, h: u32, seed: u32) -> Frame {
    let mut f = Frame::new(w, h, layout.kind(), layout.bit_depth()).expect("new frame");
    let mut x = seed | 1;
    let max = (1u32 << layout.bit_depth()) - 1;
    for p in &mut f.planes {
        for v in p.iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *v = ((x >> 7) & max) as u16;
        }
    }
    f
}

#[test]
fn pack_then_unpack_returns_the_frame() {
    // widths that are, and are not, multiples of a v210 group (6 pixels) and of its 48-pixel row unit
    for layout in LAYOUTS {
        for (w, h) in [(2, 1), (6, 2), (8, 3), (50, 4), (96, 2), (98, 3), (130, 1)] {
            let f = random_frame(layout, w, h, 7 + w);
            let bytes = encode(layout, &f).expect("encode");
            assert_eq!(bytes.len(), layout.frame_bytes(w, h).expect("size"));
            assert_eq!(decode(layout, w, h, &bytes).expect("decode"), f, "{layout:?} {w}x{h}");
            // longer data: only the first picture is read
            let mut longer = bytes.clone();
            longer.extend([0xAA; 17]);
            assert_eq!(decode(layout, w, h, &longer).expect("decode"), f);
        }
    }
}

#[test]
fn the_extremes_of_every_field_survive() {
    for layout in LAYOUTS {
        let mut f = Frame::new(6, 1, layout.kind(), layout.bit_depth()).expect("new frame");
        let max = (1u16 << layout.bit_depth()) - 1;
        for (i, p) in f.planes.iter_mut().enumerate() {
            for (k, v) in p.iter_mut().enumerate() {
                *v = if (i + k) % 2 == 0 { max } else { 0 };
            }
        }
        assert_eq!(decode(layout, 6, 1, &encode(layout, &f).expect("encode")).expect("decode"), f, "{layout:?}");
    }
}

#[test]
fn known_bytes() {
    // UYVY: U Y0 V Y1
    let f = decode(Layout::Uyvy, 2, 1, &[10, 20, 30, 40]).expect("decode");
    assert_eq!((f.planes[0].clone(), f.planes[1].clone(), f.planes[2].clone()), (vec![20, 40], vec![10], vec![30]));
    // YUYV: Y0 U Y1 V; YVYU: Y0 V Y1 U
    let f = decode(Layout::Yuyv, 2, 1, &[10, 20, 30, 40]).expect("decode");
    assert_eq!((f.planes[0].clone(), f.planes[1].clone(), f.planes[2].clone()), (vec![10, 30], vec![20], vec![40]));
    let f = decode(Layout::Yvyu, 2, 1, &[10, 20, 30, 40]).expect("decode");
    assert_eq!((f.planes[0].clone(), f.planes[1].clone(), f.planes[2].clone()), (vec![10, 30], vec![40], vec![20]));
    // R10k: R in bits 31-22; r210: R in bits 29-20
    let word = (1023u32 << 22) | (512 << 12) | (1 << 2);
    let f = decode(Layout::R10k, 1, 1, &word.to_be_bytes()).expect("decode");
    assert_eq!((f.planes[0][0], f.planes[1][0], f.planes[2][0]), (1023, 512, 1));
    let word = (1023u32 << 20) | (512 << 10) | 1;
    let f = decode(Layout::R210, 1, 1, &[&word.to_be_bytes()[..], &[0; 252]].concat()).expect("decode");
    assert_eq!((f.planes[0][0], f.planes[1][0], f.planes[2][0]), (1023, 512, 1));
    // v210: the first word is Cb0 | Y0 << 10 | Cr0 << 20
    let mut block = [0u8; 128];
    block[..4].copy_from_slice(&(100u32 | (200 << 10) | (300 << 20)).to_le_bytes());
    let f = decode(Layout::V210, 2, 1, &block).expect("decode");
    assert_eq!((f.planes[0][0], f.planes[1][0], f.planes[2][0]), (200, 100, 300));
}

#[test]
fn bad_sizes_data_and_frames_are_errors() {
    assert!(matches!(decode(Layout::Uyvy, 3, 1, &[0; 100]), Err(Error::Size { .. })));
    assert!(matches!(decode(Layout::Uyvy, 0, 1, &[0; 100]), Err(Error::Size { .. })));
    assert!(matches!(decode(Layout::R10k, 4, 0, &[0; 100]), Err(Error::Size { .. })));
    assert!(matches!(decode(Layout::R10k, 1 << 20, 1, &[]), Err(Error::Size { .. })));
    assert!(matches!(decode(Layout::V210, 6, 1, &[0; 127]), Err(Error::Short { got: 127, need: 128 })));
    for layout in LAYOUTS {
        let ok = random_frame(layout, 6, 2, 3);
        assert!(encode(layout, &ok).is_ok());
        let mut bad = ok.clone();
        bad.planes[0].pop();
        assert!(matches!(encode(layout, &bad), Err(Error::Frame(_))), "{layout:?}");
        let mut bad = ok.clone();
        bad.planes[1][0] = 1 << layout.bit_depth();
        assert!(matches!(encode(layout, &bad), Err(Error::Frame(_))), "{layout:?}");
        let mut bad = ok.clone();
        bad.bit_depth = if layout.bit_depth() == 8 { 10 } else { 8 };
        assert!(matches!(encode(layout, &bad), Err(Error::Frame(_))), "{layout:?}");
        let mut bad = ok.clone();
        bad.kind = if layout.kind() == Kind::Rgb { Kind::Yuv422 } else { Kind::Rgb };
        assert!(matches!(encode(layout, &bad), Err(Error::Frame(_))), "{layout:?}");
        // every truncation is an error, never a panic
        let bytes = encode(layout, &ok).expect("encode");
        for n in 0..bytes.len() {
            assert!(decode(layout, 6, 2, &bytes[..n]).is_err(), "{layout:?} {n}");
        }
    }
    // an odd width for a 4:2:2 frame
    let odd = Frame { width: 3, height: 1, kind: Kind::Yuv422, bit_depth: 8, planes: [vec![0; 3], vec![0; 1], vec![0; 1]] };
    assert!(matches!(encode(Layout::Uyvy, &odd), Err(Error::Size { .. })));
    // noise decodes to something and re-encodes within the bit depth
    let mut x = 5u32;
    let noise: Vec<u8> = (0..4096)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    for layout in LAYOUTS {
        let f = decode(layout, 18, 4, &noise).expect("decode noise");
        assert!(encode(layout, &f).is_ok());
    }
}

// ---- ffmpeg as the reference
//
// The oracle pictures have even heights: ffmpeg's v210 decoder returns zeros for an odd height.

fn ffmpeg(ff: &Path, args: &[&str]) {
    let out = Command::new(ff).args(["-v", "error", "-nostdin", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn s(p: &Path) -> String {
    p.to_str().expect("path").to_string()
}

/// The planes of a planar raw file as `u16` (8-bit samples are bytes, 10-bit ones little-endian words).
fn read_planes(path: &Path, w: usize, h: usize, kind: Kind, depth: u8) -> [Vec<u16>; 3] {
    let bytes = std::fs::read(path).expect("read");
    let samples: Vec<u16> = if depth == 8 {
        bytes.iter().map(|&b| u16::from(b)).collect()
    } else {
        bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect()
    };
    let cw = if kind == Kind::Yuv422 { w / 2 } else { w };
    let (a, rest) = samples.split_at(w * h);
    let (b, c) = rest.split_at(cw * h);
    assert_eq!(c.len(), cw * h, "plane sizes");
    [a.to_vec(), b.to_vec(), c.to_vec()]
}

/// (layout, ffmpeg codec, pixel format of the packed picture, planar format to compare in).
const CASES: [(Layout, &str, &str, &str); 6] = [
    (Layout::Uyvy, "rawvideo", "uyvy422", "yuv422p"),
    (Layout::Yuyv, "rawvideo", "yuyv422", "yuv422p"),
    (Layout::Yvyu, "rawvideo", "yvyu422", "yuv422p"),
    (Layout::V210, "v210", "yuv422p10le", "yuv422p10le"),
    (Layout::R210, "r210", "gbrp10le", "gbrp10le"),
    (Layout::R10k, "r10k", "gbrp10le", "gbrp10le"),
];

#[test]
fn we_unpack_and_repack_what_ffmpeg_packs() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("rawvideo");
    for (w, h) in [(50usize, 6usize), (98, 4)] {
        for (layout, codec, pix, planar) in CASES {
            let name = format!("{layout:?}_{w}x{h}");
            let nut = dir.join(format!("{name}.nut"));
            let src = format!("gradients=size={w}x{h}:rate=1");
            ffmpeg(&ff, &["-f", "lavfi", "-i", &src, "-frames:v", "1", "-c:v", codec, "-pix_fmt", pix, "-f", "nut", &s(&nut)]);
            // the packet ffmpeg wrote, and ffmpeg's own unpacking of it
            let raw = dir.join(format!("{name}.raw"));
            ffmpeg(&ff, &["-i", &s(&nut), "-c", "copy", "-f", "rawvideo", &s(&raw)]);
            let reference = dir.join(format!("{name}.planar"));
            ffmpeg(&ff, &["-i", &s(&nut), "-pix_fmt", planar, "-f", "rawvideo", &s(&reference)]);
            let packet = std::fs::read(&raw).expect("read");
            assert_eq!(packet.len(), layout.frame_bytes(w as u32, h as u32).expect("size"), "{name}: packet size");
            let ours = decode(layout, w as u32, h as u32, &packet).expect("decode");
            let mut theirs = read_planes(&reference, w, h, layout.kind(), layout.bit_depth());
            if planar == "gbrp10le" {
                // ffmpeg's planar RGB is G, B, R
                theirs = [theirs[2].clone(), theirs[0].clone(), theirs[1].clone()];
            }
            assert!(ours.planes == theirs, "{name}: our unpacking differs from ffmpeg's");
            assert!(encode(layout, &ours).expect("encode") == packet, "{name}: our packing differs from ffmpeg's");
        }
    }
}

#[test]
fn ffmpeg_unpacks_what_we_pack() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("rawvideo");
    // the layouts ffmpeg reads from a bare file: the raw pixel formats and v210
    for (layout, fmt_args, planar) in [
        (Layout::Uyvy, vec!["-f", "rawvideo", "-pix_fmt", "uyvy422"], "yuv422p"),
        (Layout::Yuyv, vec!["-f", "rawvideo", "-pix_fmt", "yuyv422"], "yuv422p"),
        (Layout::Yvyu, vec!["-f", "rawvideo", "-pix_fmt", "yvyu422"], "yuv422p"),
        (Layout::V210, vec!["-f", "v210"], "yuv422p10le"),
    ] {
        for (w, h) in [(50u32, 6u32), (98, 4)] {
            let f = random_frame(layout, w, h, 11);
            let name = format!("ours_{layout:?}_{w}x{h}");
            let packed = dir.join(format!("{name}.raw"));
            std::fs::write(&packed, encode(layout, &f).expect("encode")).expect("write");
            let out = dir.join(format!("{name}.planar"));
            let size = format!("{w}x{h}");
            let mut args: Vec<&str> = fmt_args.clone();
            args.extend(["-video_size", &size, "-i"]);
            let input = s(&packed);
            args.push(&input);
            args.extend(["-pix_fmt", planar, "-frames:v", "1", "-f", "rawvideo"]);
            let output = s(&out);
            args.push(&output);
            ffmpeg(&ff, &args);
            let theirs = read_planes(&out, w as usize, h as usize, layout.kind(), layout.bit_depth());
            assert!(theirs == f.planes, "{name}: ffmpeg unpacks our packing differently");
        }
    }
}

#[test]
fn frame_new_validates_dimensions_and_rejects_oversized_allocation() {
    assert!(Frame::new(u32::MAX, u32::MAX, Kind::Rgb, 10).is_err());
    assert!(Frame::new(0, 100, Kind::Rgb, 8).is_err());
    assert!(Frame::new(100, 0, Kind::Rgb, 8).is_err());
    assert!(Frame::new(3, 100, Kind::Yuv422, 8).is_err());
    assert!(Frame::new(100, 100, Kind::Rgb, 12).is_err());
}
