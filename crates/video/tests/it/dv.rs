//! DV decoding against ffmpeg as the reference (oracle; skipped when ffmpeg is absent, a failure with
//! `CRAFT_MEDIA_REQUIRE_ORACLES=1`): ffmpeg-made 25 Mb/s DV (525/60 4:1:1, 625/50 4:2:0) and DVCPRO50 (4:2:2), with
//! and without the 2-4-8 DCT, decode to ffmpeg's pictures in their own sampling. DV leaves the inverse DCT's
//! arithmetic to the decoder, so the planes are compared by PSNR. And damaged frames: errors or pictures, never a
//! panic.

use craft_media_testkit::fixtures;
use craft_video::dv::{self, Layout, Sampling, System};
use craft_video::frame::VideoFrame;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A raw DV file of two seconds of moving test picture (and a stereo tone) made by ffmpeg in `pixels`. `interlaced`:
/// fields of a picture at twice the rate (moving between them), coded with the 2-4-8 DCT where the encoder finds it
/// better.
fn dv_file(ff: &Path, name: &str, size: &str, rate: &str, pixels: &str, aspect: &str, interlaced: bool) -> Option<PathBuf> {
    let out = craft_media_testkit::fixture_subdir("dv").join(name);
    fixtures::generate(&out, |tmp| {
        let (source, filter) = if interlaced {
            (format!("testsrc2=size={size}:rate={rate}*2"), "[0]tinterlace=mode=interleave_bottom,setfield=bff[v]")
        } else {
            (format!("testsrc2=size={size}:rate={rate}"), "[0]null[v]")
        };
        let mut cmd = Command::new(ff);
        cmd.args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"]).arg(source);
        cmd.args(["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"]);
        cmd.args(["-filter_complex", filter, "-map", "[v]", "-map", "1:a", "-t", "2"]);
        cmd.args(["-c:v", "dvvideo", "-pix_fmt", pixels, "-aspect", aspect]);
        if interlaced {
            cmd.args(["-flags", "+ildct"]);
        }
        cmd.args(["-c:a", "pcm_s16le", "-ac", "2", "-f", "dv"]).arg(tmp);
        cmd.status().is_ok_and(|s| s.success())
    })
}

/// ffmpeg's pictures of `path`: raw planar frames in `pixels`.
fn ffmpeg_pictures(ff: &Path, path: &Path, pixels: &str) -> Vec<u8> {
    let out = Command::new(ff)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:v", "-f", "rawvideo", "-pix_fmt", pixels, "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

/// How many DCT blocks of `frame` are coded 2-4-8: the mode bit after the 9-bit DC at the start of each block's
/// area in each video DIF block (BT.1618-1 Figs 29 and 30).
fn blocks_248(frame: &[u8], system: System, sampling: Sampling) -> usize {
    let (channels, areas): (usize, &[usize]) = if sampling == Sampling::C422 { (2, &[8, 232, 456, 536]) } else { (1, &[8, 120, 232, 344, 456, 536]) };
    let mut count = 0;
    for sequence in 0..system.sequences() * channels {
        for block in 0..150 {
            let start = sequence * dv::SEQUENCE + block * dv::DIF_BLOCK;
            if frame[start] >> 5 != 4 {
                continue;
            }
            for &area in areas {
                let bit = area + 9;
                if frame[start + 3 + bit / 8] >> (7 - bit % 8) & 1 == 1 {
                    count += 1;
                }
            }
        }
    }
    count
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let error = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum::<f64>() / a.len() as f64;
    if error == 0.0 { 99.0 } else { 10.0 * (255.0f64.powi(2) / error).log10() }
}

/// Decodes every frame of `path` and compares it with ffmpeg's in the frame's own sampling; returns how many
/// blocks were coded 2-4-8.
fn compare(ff: &Path, path: &Path, system: System, sampling: Sampling, wide: bool) -> usize {
    let bytes = std::fs::read(path).expect("read the DV file");
    let channels = if sampling == Sampling::C422 { 2 } else { 1 };
    let frame_bytes = system.frame_bytes() * channels;
    assert_eq!(bytes.len() % frame_bytes, 0, "whole frames");
    let (pixels, layout) = match sampling {
        Sampling::C411 => ("yuv411p", Layout::Yuv411),
        Sampling::C420 => ("yuv420p", Layout::Yuv420),
        Sampling::C422 => ("yuv422p", Layout::Yuv422),
    };
    let theirs = ffmpeg_pictures(ff, path, pixels);
    let frames = bytes.len() / frame_bytes;
    let mut lowest = f64::MAX;
    let mut field_blocks = 0;
    for (index, frame) in bytes.chunks_exact(frame_bytes).enumerate() {
        assert!(dv::sniff(frame));
        let header = dv::header(frame).expect("header");
        assert_eq!((header.system, header.sampling, header.wide), (system, sampling, wide));
        field_blocks += blocks_248(frame, system, sampling);
        let picture = dv::decode(frame).expect("decode");
        assert_eq!(picture.layout, layout);
        let (cw, ch) = picture.chroma_size();
        let size = picture.width * picture.height + 2 * cw * ch;
        assert_eq!(theirs.len(), frames * size, "ffmpeg's pictures are as large as ours");
        let theirs = &theirs[index * size..(index + 1) * size];
        let (y, rest) = theirs.split_at(picture.width * picture.height);
        let (cb, cr) = rest.split_at(cw * ch);
        let (py, pb, pr) = (psnr(&picture.y, y), psnr(&picture.cb, cb), psnr(&picture.cr, cr));
        assert!(py > 45.0 && pb > 45.0 && pr > 45.0, "frame {index}: Y {py:.1} Cb {pb:.1} Cr {pr:.1} dB");
        lowest = lowest.min(py).min(pb).min(pr);
        // The picture is a codec-agnostic frame of the same planes.
        let (width, chroma) = (picture.width, cw);
        let video = VideoFrame::try_from(picture).expect("a video frame");
        assert_eq!(video.planes()[0].width() as usize, width);
        assert_eq!(video.planes()[1].width() as usize, chroma);
    }
    eprintln!("{}: {frames} frames, {field_blocks} blocks 2-4-8, lowest PSNR {lowest:.1} dB", path.display());
    field_blocks
}

#[test]
fn twenty_five_megabit_525_line_dv_decodes_as_ffmpeg_decodes_it() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let Some(path) = dv_file(&ff, "ntsc.dv", "720x480", "30000/1001", "yuv411p", "4:3", false) else { return };
    compare(&ff, &path, System::Lines525, Sampling::C411, false);
    // Interlaced, so the 2-4-8 DCT is used.
    let Some(path) = dv_file(&ff, "ntsc_fields.dv", "720x480", "30000/1001", "yuv411p", "4:3", true) else { return };
    let field_blocks = compare(&ff, &path, System::Lines525, Sampling::C411, false);
    assert!(field_blocks > 1_000, "{field_blocks} blocks coded 2-4-8");
}

#[test]
fn twenty_five_megabit_625_line_dv_decodes_as_ffmpeg_decodes_it() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let Some(path) = dv_file(&ff, "pal_wide.dv", "720x576", "25", "yuv420p", "16:9", true) else { return };
    let field_blocks = compare(&ff, &path, System::Lines625, Sampling::C420, true);
    assert!(field_blocks > 1_000, "{field_blocks} blocks coded 2-4-8");
}

/// DVCPRO50: two channels of DIF sequences, the second coding the odd super block rows, and an additional AC area
/// after each Y block.
#[test]
fn fifty_megabit_dv_decodes_as_ffmpeg_decodes_it() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let Some(path) = dv_file(&ff, "dv50_ntsc.dv", "720x480", "30000/1001", "yuv422p", "4:3", true) else { return };
    let field_blocks = compare(&ff, &path, System::Lines525, Sampling::C422, false);
    assert!(field_blocks > 1_000, "{field_blocks} blocks coded 2-4-8");
    let Some(path) = dv_file(&ff, "dv50_pal_wide.dv", "720x576", "25", "yuv422p", "16:9", false) else { return };
    compare(&ff, &path, System::Lines625, Sampling::C422, true);
}

/// Every truncation and seeded damage of a real frame: a picture or an error, never a panic; a frame cut short of
/// its DIF sequences is an error.
#[test]
fn damaged_frames_are_errors_or_pictures() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let Some(path) = dv_file(&ff, "ntsc.dv", "720x480", "30000/1001", "yuv411p", "4:3", false) else { return };
    let bytes = std::fs::read(path).expect("read the DV file");
    let frame = bytes[..System::Lines525.frame_bytes()].to_vec();
    for cut in [0, 7, dv::DIF_BLOCK, 1_000, frame.len() - 1] {
        assert!(dv::decode(&frame[..cut]).is_err(), "a frame cut to {cut} bytes");
    }
    for value in [0x00u8, 0xFF, 0x55] {
        // The header block is kept, so the frame is still taken for DV.
        let mut flat = frame.clone();
        flat[dv::DIF_BLOCK..].fill(value);
        let _ = dv::decode(&flat);
    }
    craft_media_testkit::hostile::hammer("dv", &frame, 64, |data| {
        let _ = dv::decode(data);
    });
}
