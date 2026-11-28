//! NV12 and P010 input against ffmpeg's `nv12` and `p010le` layouts (skipped when ffmpeg is absent):
//! pictures ffmpeg converts from planar 4:2:0 code the same stream as the planar pictures.

use super::common;

use common::{Yuv, Yuv16, ffmpeg, le16, synth, synth10};
use craft_video::h264::*;
use std::process::Command;

/// `raw` (frames of `from`) converted by ffmpeg to frames of `to`.
fn convert(raw: &[u8], w: usize, h: usize, from: &str, to: &str) -> Vec<u8> {
    let dir = common::out_dir("semi_planar");
    let src = dir.join(format!("{from}.raw"));
    std::fs::write(&src, raw).unwrap();
    let out = Command::new(ffmpeg().unwrap())
        .args(["-nostdin", "-v", "error", "-f", "rawvideo", "-pix_fmt", from, "-s", &format!("{w}x{h}"), "-i"])
        .arg(&src)
        .args(["-f", "rawvideo", "-pix_fmt", to, "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn stream(cfg: &EncoderConfig, frames: usize, mut encode: impl FnMut(&mut Encoder, usize) -> Vec<Packet>) -> Vec<u8> {
    let mut enc = Encoder::new(cfg.clone()).unwrap();
    let mut packets: Vec<Packet> = (0..frames).flat_map(|i| encode(&mut enc, i)).collect();
    packets.extend(enc.flush());
    assert_eq!(packets.len(), frames);
    packets.iter().flat_map(|p| p.data.iter().copied()).collect()
}

#[test]
fn ffmpeg_nv12_pictures_code_the_same_stream_as_planar_pictures() {
    if ffmpeg().is_none() {
        return;
    }
    let (w, h, n) = (96, 64, 5);
    let frames: Vec<Yuv> = (0..n).map(|t| synth(w, h, t)).collect();
    let raw: Vec<u8> = frames.iter().flat_map(Yuv::bytes).collect();
    let nv12 = convert(&raw, w, h, "yuv420p", "nv12");
    let size = w * h * 3 / 2;
    assert_eq!(nv12.len(), size * n);
    let cfg = EncoderConfig::new(w as u32, h as u32, 30, 1);
    let planar = stream(&cfg, n, |enc, i| enc.encode(&frames[i].frame(), i as i64).unwrap());
    let semi = stream(&cfg, n, |enc, i| {
        let pic = &nv12[i * size..(i + 1) * size];
        let (y, uv) = pic.split_at(w * h);
        enc.encode_nv12(&Nv12Frame { y, uv, y_stride: w, uv_stride: w }, i as i64).unwrap()
    });
    assert!(planar == semi, "the NV12 stream differs from the planar one");
}

#[test]
fn ffmpeg_p010_pictures_code_the_same_stream_as_16_bit_pictures() {
    if ffmpeg().is_none() {
        return;
    }
    let (w, h, n) = (96, 64, 5);
    let frames: Vec<Yuv16> = (0..n).map(|t| synth10(w, h, t)).collect();
    let raw: Vec<u8> = frames.iter().flat_map(Yuv16::bytes).collect();
    let p010 = le16(&convert(&raw, w, h, "yuv420p10le", "p010le"));
    let size = w * h * 3 / 2;
    assert_eq!(p010.len(), size * n);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 30, 1);
    cfg.profile = Profile::High10;
    cfg.bit_depth = 10;
    let deep = stream(&cfg, n, |enc, i| enc.encode16(&frames[i].frame(), i as i64).unwrap());
    let semi = stream(&cfg, n, |enc, i| {
        let pic = &p010[i * size..(i + 1) * size];
        let (y, uv) = pic.split_at(w * h);
        enc.encode_p010(&P010Frame { y, uv, y_stride: w, uv_stride: w }, i as i64).unwrap()
    });
    assert!(deep == semi, "the P010 stream differs from the 16-bit one");
}
