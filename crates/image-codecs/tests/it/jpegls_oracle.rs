//! JPEG-LS against ffmpeg as the reference (oracle; skipped when ffmpeg is absent, a failure with
//! `CRAFT_MEDIA_REQUIRE_ORACLES=1`):
//! - files ffmpeg's encoder writes (8- and 16-bit gray: smooth, noisy and flat images, which
//!   exercise the regular, run and escape-coded paths) decode to the pixels ffmpeg decodes;
//! - files our encoder writes (gray 8 and 16 bits, RGB 8 bits) decode in ffmpeg to our pixels (ffmpeg
//!   cannot decode 16-bit RGB JPEG-LS, so that case is only round-tripped through our own decoder);
//! - ffmpeg writes RGB line-interleaved, which is reported as unsupported.

use craft_image_codecs::{ChannelLayout, CodecError, EncodeOptions, Format, Image, SampleType, decode, encode};
use std::path::Path;
use std::process::Command;

const W: u32 = 66;
const H: u32 = 40;

fn run(ffmpeg: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new(ffmpeg).args(["-v", "error", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

/// `(name, lavfi source)`: smooth, noisy and flat pictures of W x H.
fn sources() -> Vec<(&'static str, String)> {
    let s = format!("{W}x{H}");
    vec![
        ("testsrc", format!("testsrc2=size={s}:rate=1")),
        ("noise", format!("nullsrc=size={s}:rate=1,geq=lum='random(1)*255':cb=128:cr=128")),
        ("flat", format!("color=c=0x336699:size={s}:rate=1")),
        ("gradient", format!("gradients=size={s}:rate=1:seed=7")),
    ]
}

#[test]
fn ffmpeg_files_decode_to_ffmpegs_pixels() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("jpegls");
    for (name, src) in sources() {
        for (fmt, ext_bits) in [("gray", 8), ("gray16le", 16)] {
            let file = dir.join(format!("ffmpeg_{name}_{ext_bits}.jls"));
            let f = file.to_str().expect("path");
            run(&ffmpeg, &["-f", "lavfi", "-i", &src, "-frames:v", "1", "-pix_fmt", fmt, f]);
            let theirs = run(&ffmpeg, &["-i", f, "-f", "rawvideo", "-pix_fmt", fmt, "-"]);
            let ours = decode(&std::fs::read(&file).expect("read")).unwrap_or_else(|e| panic!("{name} {fmt}: {e}"));
            assert_eq!((ours.width(), ours.height(), ours.layout()), (W, H, ChannelLayout::Gray), "{name} {fmt}");
            let want_sample = if ext_bits == 8 { SampleType::U8 } else { SampleType::U16 };
            assert_eq!(ours.sample_type(), want_sample);
            let ours_bytes: Vec<u8> = if ext_bits == 8 {
                ours.data().to_vec()
            } else {
                ours.data().as_chunks::<2>().0.iter().flat_map(|b| u16::from_ne_bytes(*b).to_le_bytes()).collect()
            };
            assert!(ours_bytes == theirs, "{name} {fmt}: pixels differ from ffmpeg's decode");
        }
    }
}

#[test]
fn ffmpegs_rgb_is_line_interleaved_and_unsupported() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("jpegls");
    let file = dir.join("ffmpeg_rgb.jls");
    run(
        &ffmpeg,
        &["-f", "lavfi", "-i", &format!("testsrc2=size={W}x{H}:rate=1"), "-frames:v", "1", "-pix_fmt", "rgb24", file.to_str().expect("path")],
    );
    assert!(matches!(decode(&std::fs::read(&file).expect("read")), Err(CodecError::Unsupported { .. })));
}

#[test]
fn ffmpeg_decodes_our_files_to_our_pixels() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("jpegls");
    for (name, src) in sources() {
        for (layout, fmt8, fmt16) in [(ChannelLayout::Gray, "gray", "gray16le"), (ChannelLayout::Rgb, "rgb24", "rgb48le")] {
            for bits in [8, 16] {
                let fmt = if bits == 8 { fmt8 } else { fmt16 };
                // Source pixels from ffmpeg, so the test image is realistic.
                let raw = run(&ffmpeg, &["-f", "lavfi", "-i", &src, "-frames:v", "1", "-pix_fmt", fmt, "-f", "rawvideo", "-"]);
                let img = if bits == 8 {
                    Image::from_u8(W, H, layout, raw).expect("image")
                } else {
                    let samples: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
                    Image::from_u16(W, H, layout, &samples).expect("image")
                };
                let bytes = encode(&img, Format::JpegLs, &EncodeOptions::default()).expect("encode");
                assert_eq!(decode(&bytes).expect("our own file decodes").data(), img.data(), "{name} {fmt}: round trip");
                let file = dir.join(format!("ours_{name}_{fmt}.jls"));
                std::fs::write(&file, &bytes).expect("write");
                // ffmpeg's decoder does not implement 16-bit colour ("Not yet implemented").
                if layout == ChannelLayout::Rgb && bits == 16 {
                    continue;
                }
                let theirs = run(&ffmpeg, &["-i", file.to_str().expect("path"), "-f", "rawvideo", "-pix_fmt", fmt, "-"]);
                let ours_bytes: Vec<u8> = if bits == 8 {
                    img.data().to_vec()
                } else {
                    img.data().as_chunks::<2>().0.iter().flat_map(|b| u16::from_ne_bytes(*b).to_le_bytes()).collect()
                };
                assert!(theirs == ours_bytes, "{name} {fmt}: ffmpeg decodes our file to other pixels");
            }
        }
    }
}
