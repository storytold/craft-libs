//! Interlaced output (`EncoderConfig::interlaced`): frames coded as frame pictures of an interlaced sequence, each
//! with a picture timing SEI giving the field order. ffmpeg (an external oracle) decodes them with no errors,
//! bit-identical to the encoder's reconstruction, and reports the field order; craft-video's own decoder gives the
//! same pictures.

use crate::common;

use common::{Yuv, ffmpeg, ffmpeg_decode, ffprobe, out_dir, synth};
use craft_video::h264::*;
use std::process::Command;

/// Frames whose two fields are of different moments (the top field from time `2t`, the bottom one from `2t + 1`).
fn woven(w: usize, h: usize, t: usize) -> Yuv {
    let (top, bottom) = (synth(w, h, 2 * t), synth(w, h, 2 * t + 1));
    let mut frame = top;
    for row in (1..h).step_by(2) {
        frame.y[row * w..(row + 1) * w].copy_from_slice(&bottom.y[row * w..(row + 1) * w]);
    }
    let cw = w / 2;
    for row in (1..h / 2).step_by(2) {
        frame.u[row * cw..(row + 1) * cw].copy_from_slice(&bottom.u[row * cw..(row + 1) * cw]);
        frame.v[row * cw..(row + 1) * cw].copy_from_slice(&bottom.v[row * cw..(row + 1) * cw]);
    }
    frame
}

fn run(name: &str, (w, h): (usize, usize), order: FieldOrder, expected_order: &str) {
    let (Some(_), Some(ffprobe)) = (ffmpeg(), ffprobe()) else {
        eprintln!("ffmpeg / ffprobe not found; skipping");
        return;
    };
    let frames: Vec<Yuv> = (0..8).map(|t| woven(w, h, t)).collect();
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 30_000, 1001);
    cfg.bframes = 2;
    cfg.keyint = 4;
    cfg.rate = RateControl::Vbr { target_kbps: 8_000, max_kbps: 16_000 };
    cfg.threads = 4;
    cfg.slices = 2;
    cfg.interlaced = Some(order);
    let mut enc = Encoder::new(cfg).unwrap();
    enc.set_recon_capture(true);
    let mut packets = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        packets.extend(enc.encode(&f.frame(), i as i64).unwrap());
    }
    packets.extend(enc.flush());
    let mut recon = enc.take_recon();
    recon.sort_by_key(|r| r.pts);
    let stream: Vec<u8> = packets.iter().flat_map(|p| p.data.iter().copied()).collect();

    // ffmpeg: no errors, every frame as reconstructed.
    let path = out_dir("interlaced").join(format!("{name}.h264"));
    std::fs::write(&path, &stream).unwrap();
    let (err, decoded) = ffmpeg_decode(&path);
    assert!(err.trim().is_empty(), "{name}: ffmpeg reported errors: {err}");
    let size = w * h * 3 / 2;
    assert_eq!(decoded.len(), frames.len() * size, "{name}: decoded frame count");
    assert_eq!(recon.len(), frames.len());
    for (i, r) in recon.iter().enumerate() {
        let d = &decoded[i * size..(i + 1) * size];
        assert!(d[..w * h] == r.y[..], "{name}: frame {i} luma differs from the reconstruction");
        assert!(d[w * h..w * h * 5 / 4] == r.u[..] && d[w * h * 5 / 4..] == r.v[..], "{name}: frame {i} chroma");
    }

    // ffprobe reads the stream as interlaced, in the field order given.
    let probe = Command::new(ffprobe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,field_order"])
        .args(["-of", "csv=p=0"])
        .arg(&path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&probe.stdout);
    assert_eq!(text.trim().trim_end_matches(','), format!("{w},{h},{expected_order}"), "{name}");

    // craft-video's decoder gives the same pictures.
    #[cfg(feature = "h264-decode")]
    decoded_by_craft_video(name, (w, h), &packets, &recon);
}

#[cfg(feature = "h264-decode")]
fn decoded_by_craft_video(name: &str, (w, h): (usize, usize), packets: &[Packet], recon: &[ReconFrame]) {
    let mut decoder = craft_video::h264::decode::Decoder::new();
    let mut pictures = Vec::new();
    for p in packets {
        pictures.extend(decoder.decode(&p.data, p.pts).unwrap());
    }
    pictures.extend(decoder.flush());
    assert_eq!(pictures.len(), recon.len(), "{name}");
    let mut by_pts: Vec<_> = pictures.iter().collect();
    by_pts.sort_by_key(|p| p.pts);
    for (i, (p, r)) in by_pts.iter().zip(recon).enumerate() {
        assert_eq!((p.width as usize, p.height as usize), (w, h));
        for row in 0..h {
            assert!(p.y[row * p.y_stride..row * p.y_stride + w] == r.y[row * w..(row + 1) * w], "{name}: frame {i} row {row}");
        }
    }
}

/// 576i, the top field first (PAL DV and broadcast).
#[test]
fn interlaced_576_top_field_first() {
    run("576i_tff", (720, 576), FieldOrder::TopFirst, "tt");
}

/// 1080i, the bottom field first: 1088 coded lines cropped to 1080 in units of 4 lines.
#[test]
fn interlaced_1080_bottom_field_first() {
    run("1080i_bff", (1920, 1080), FieldOrder::BottomFirst, "bb");
}

/// An interlaced picture must be whole macroblock pairs high.
#[test]
fn an_interlaced_picture_of_an_odd_number_of_macroblock_rows_is_refused() {
    let mut cfg = EncoderConfig::new(720, 480 + 16, 30_000, 1001);
    cfg.interlaced = Some(FieldOrder::TopFirst);
    assert!(Encoder::new(cfg).is_err());
    let mut cfg = EncoderConfig::new(720, 480, 30_000, 1001);
    cfg.interlaced = Some(FieldOrder::TopFirst);
    assert!(Encoder::new(cfg).is_ok());
}

/// Interlaced coding is allowed at levels 2.1 to 4.1 only (Table A-4): a small picture is said to be level 2.1, and
/// a picture that needs a level above 4.1 (1080 lines at 60 frames a second) is refused.
#[test]
fn an_interlaced_stream_keeps_to_levels_2_1_to_4_1() {
    let (w, h) = (352, 288);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 30_000, 1001);
    cfg.interlaced = Some(FieldOrder::TopFirst);
    let mut enc = Encoder::new(cfg).unwrap();
    let mut packets = enc.encode(&synth(w, h, 0).frame(), 0).unwrap();
    packets.extend(enc.flush());
    let data = &packets[0].data;
    // The sequence parameter set's NAL unit: profile_idc, the constraint flags, then level_idc.
    let sps = data.windows(4).position(|b| b[..3] == [0, 0, 1] && b[3] & 0x1f == 7).unwrap() + 4;
    assert_eq!(data[sps + 2], 21);
    let mut cfg = EncoderConfig::new(1920, 1080, 60, 1);
    cfg.interlaced = Some(FieldOrder::TopFirst);
    assert!(Encoder::new(cfg).is_err());
}
