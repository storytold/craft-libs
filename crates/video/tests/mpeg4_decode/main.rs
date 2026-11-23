//! Integration, roundtrip, hostile-input, and black-box ffmpeg oracle tests for the clean-room
//! MPEG-4 Part 2 (`mpeg4-decode`) video decoder.

use std::path::Path;
use std::process::Command;

use craft_media::{CodecId, CodecParams};
use craft_video::mpeg4::{Decoder, Picture};
use craft_video::{TrackDecoder, VideoError};

fn picture_bytes(pic: &Picture) -> Vec<u8> {
    [&pic.y[..], &pic.u[..], &pic.v[..]].concat()
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "frame byte length mismatch");
    let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    if se == 0.0 {
        return 99.0;
    }
    10.0 * (255.0f64 * 255.0 * a.len() as f64 / se).log10()
}

#[cfg(feature = "mpeg4-encode")]
mod roundtrip {
    use super::*;
    use craft_video::frame::{ColorFamily, FieldOrder, Samples, Subsampling, VideoFrame};
    use craft_video::mpeg4::{ColorConfig, Encoder, EncoderConfig, RateControl, VopType, YuvFrame};

    fn synth_frame(w: usize, h: usize, t: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let tf = t as f32;
        let mut y = vec![0u8; w * h];
        for yy in 0..h {
            for xx in 0..w {
                let (px, py) = (xx as f32 + tf * 2.0, yy as f32 + tf * 1.0);
                let v = 128.0 + 45.0 * (px / 8.0).sin() + 35.0 * (py / 6.0).cos();
                y[yy * w + xx] = v.clamp(0.0, 255.0) as u8;
            }
        }
        let (cw, ch) = (w / 2, h / 2);
        let mut u = vec![0u8; cw * ch];
        let mut v = vec![0u8; cw * ch];
        for yy in 0..ch {
            for xx in 0..cw {
                u[yy * cw + xx] = (128.0 + 40.0 * ((xx as f32 + tf) / 5.0).sin()) as u8;
                v[yy * cw + xx] = (128.0 + 40.0 * ((yy as f32 - tf) / 5.0).cos()) as u8;
            }
        }
        (y, u, v)
    }

    #[test]
    fn decoder_reproduces_encoder_reconstruction_byte_for_byte() {
        for (w, h, q) in [(160, 120, 2), (98, 66, 5), (18, 34, 12), (2, 2, 1), (64, 48, 31)] {
            let mut cfg = EncoderConfig::new(w, h, 25, 1);
            cfg.rate = RateControl::Quant(q);
            cfg.keyint = 5;
            cfg.sar = (12, 11);
            cfg.color = Some(ColorConfig { primaries: 1, transfer: 1, matrix: 1, full_range: false });
            let mut enc = Encoder::new(cfg).expect("encoder");
            let mut dec = Decoder::new();

            for t in 0..12 {
                let (y, u, v) = synth_frame(w as usize, h as usize, t);
                let frame = YuvFrame { y: &y, u: &u, v: &v, y_stride: w as usize, uv_stride: (w / 2) as usize };
                let pkt = enc.encode(&frame).expect("encode");
                let recon = enc.reconstruction().expect("reconstruction");
                let pics = dec.decode(&pkt.data, pkt.pts).expect("decode");
                assert_eq!(pics.len(), 1, "{w}x{h} frame {t}");
                let pic = &pics[0];
                assert_eq!((pic.width, pic.height), (w, h));
                assert_eq!(pic.pts, pkt.pts);
                assert_eq!(pic.vop_type, if pkt.keyframe { VopType::I } else { VopType::P });
                assert_eq!(pic.y, recon.y, "{w}x{h} frame {t} Y mismatch");
                assert_eq!(pic.u, recon.u, "{w}x{h} frame {t} U mismatch");
                assert_eq!(pic.v, recon.v, "{w}x{h} frame {t} V mismatch");

                let vf = VideoFrame::try_from(pic.clone()).expect("convert to VideoFrame");
                assert_eq!((vf.width(), vf.height()), (w, h));
                assert_eq!(vf.family(), ColorFamily::YCbCr);
                assert_eq!(vf.subsampling(), Subsampling::S420);
                assert_eq!(vf.field_order, FieldOrder::Progressive);
                assert_eq!(vf.sar, Some((12, 11)));
                assert_eq!((vf.color.primaries, vf.color.transfer, vf.color.matrix), (1, 1, 1));
            }
            assert!(dec.flush().is_empty());
        }
    }

    #[test]
    fn track_decoder_decodes_out_of_band_extradata_packets() {
        let (w, h) = (128, 96);
        let mut cfg = EncoderConfig::new(w, h, 30000, 1001);
        cfg.rate = RateControl::Quant(4);
        cfg.headers_in_band = false;
        let mut enc = Encoder::new(cfg).expect("encoder");
        let params = enc.codec_params();
        assert_eq!(params.codec, CodecId::Mpeg4Video);
        assert!(!params.extradata.is_empty());

        let mut track_dec = TrackDecoder::new(&params).expect("TrackDecoder for Mpeg4Video");
        assert_eq!(track_dec.codec(), CodecId::Mpeg4Video);

        for t in 0..8 {
            let (y, u, v) = synth_frame(w as usize, h as usize, t);
            let frame = YuvFrame { y: &y, u: &u, v: &v, y_stride: w as usize, uv_stride: (w / 2) as usize };
            let pkt = enc.encode(&frame).expect("encode");
            let recon = enc.reconstruction().expect("recon");
            let frames = track_dec.decode(&pkt.data, pkt.pts * 1001).expect("track decode");
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].pts, Some(pkt.pts * 1001));
            assert_eq!(frames[0].planes()[0].samples(), &Samples::U8(recon.y));
        }

        track_dec.reset().expect("reset keeps extradata VOL");
        let (y, u, v) = synth_frame(w as usize, h as usize, 0);
        enc.request_keyframe();
        let pkt = enc.encode(&YuvFrame { y: &y, u: &u, v: &v, y_stride: w as usize, uv_stride: (w / 2) as usize }).expect("encode key");
        let frames = track_dec.decode(&pkt.data, 999).expect("decode after reset");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].pts, Some(999));
    }

    #[test]
    fn hostile_corruptions_never_panic() {
        let mut cfg = EncoderConfig::new(64, 48, 25, 1);
        cfg.rate = RateControl::Quant(6);
        let mut enc = Encoder::new(cfg).expect("encoder");
        let (y, u, v) = synth_frame(64, 48, 0);
        let pkt0 = enc.encode(&YuvFrame { y: &y, u: &u, v: &v, y_stride: 64, uv_stride: 32 }).expect("pkt0");
        let (y1, u1, v1) = synth_frame(64, 48, 1);
        let pkt1 = enc.encode(&YuvFrame { y: &y1, u: &u1, v: &v1, y_stride: 64, uv_stride: 32 }).expect("pkt1");

        // Truncations at every byte position
        for len in 1..pkt0.data.len() {
            let mut dec = Decoder::new();
            let _ = dec.decode(&pkt0.data[..len], 0);
        }

        // Single-byte bit flips across both I and P packets
        for pos in (0..pkt0.data.len()).step_by(3) {
            let mut corrupted = pkt0.data.clone();
            corrupted[pos] ^= 0x5A;
            let mut dec = Decoder::new();
            let _ = dec.decode(&corrupted, 0);
        }
        for pos in (0..pkt1.data.len()).step_by(3) {
            let mut dec = Decoder::new();
            let _ = dec.decode(&pkt0.data, 0);
            let mut corrupted = pkt1.data.clone();
            corrupted[pos] ^= 0xA5;
            let _ = dec.decode(&corrupted, 1);
            let _ = dec.flush();
        }
    }
}

#[test]
fn memory_budget_refuses_oversized_vol() {
    let mut dec = Decoder::new();
    dec.set_memory_budget(1024); // Too small for any non-trivial frame
    // Build a valid VOL header for 320x240
    let vol_stream = [
        0x00, 0x00, 0x01, 0xB0, 0x03, // VOS
        0x00, 0x00, 0x01, 0x20, // VOL
        0x00, 0x84, 0x40, 0xFA, 0x28, 0x50, 0x21, 0xE0, 0xA2, 0x1F,
    ];
    assert!(dec.decode(&vol_stream, 0).is_err());
}

#[test]
fn track_decoder_rejects_picture_in_extradata() {
    let params = CodecParams::new(CodecId::Mpeg4Video).with_extradata(vec![0, 0, 1, 0xB6, 0, 0]);
    assert!(matches!(TrackDecoder::new(&params), Err(VideoError::Malformed(_))));
}

/// Encode a synthetic test sequence with external `ffmpeg` using the given codec arguments,
/// decode it with both `ffmpeg` and our clean-room `Decoder`, and verify that every frame
/// matches within IDCT tolerance (`PSNR >= min_psnr`).
fn check_ffmpeg_stream(name: &str, w: u32, h: u32, frames: usize, extra_args: &[&str], min_psnr: f64) {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("mpeg4 decode oracle") else { return };
    let dir = craft_media_testkit::private_dir(&format!("mpeg4dec-{name}"));
    let stream_path = dir.join("stream.m4v");

    let mut cmd = Command::new(&ff);
    cmd.args([
        "-hide_banner",
        "-nostdin",
        "-y",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=size={w}x{h}:rate=25"),
        "-frames:v",
        &frames.to_string(),
        "-pix_fmt",
        "yuv420p",
    ]);
    cmd.args(extra_args);
    cmd.arg(&stream_path);
    let enc_out = cmd.output().expect("run ffmpeg encode");
    assert!(enc_out.status.success(), "{name} encode failed: {}", String::from_utf8_lossy(&enc_out.stderr));

    let ref_out = Command::new(&ff)
        .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
        .arg(&stream_path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .expect("run ffmpeg decode");
    assert!(ref_out.status.success(), "{name} ref decode failed: {}", String::from_utf8_lossy(&ref_out.stderr));

    let bit_data = std::fs::read(&stream_path).expect("read stream");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dec = Decoder::new();
    let mut got_pics = dec.decode(&bit_data, 0).expect("clean-room decode");
    got_pics.extend(dec.flush());

    let frame_bytes = (w as usize) * (h as usize) * 3 / 2;
    assert_eq!(ref_out.stdout.len(), frame_bytes * frames, "{name}: reference byte count");
    assert_eq!(got_pics.len(), frames, "{name}: decoded picture count");

    for (i, (pic, ref_frame)) in got_pics.iter().zip(ref_out.stdout.chunks_exact(frame_bytes)).enumerate() {
        let ours = picture_bytes(pic);
        let p = psnr(&ours, ref_frame);
        assert!(p >= min_psnr, "{name}: frame {i} PSNR {p:.2} dB < {min_psnr:.2} dB (type {:?})", pic.vop_type);
    }
}

#[test]
fn decodes_ffmpeg_simple_and_4mv_streams() {
    check_ffmpeg_stream("sp_basic", 176, 144, 6, &["-c:v", "mpeg4", "-q:v", "3", "-bf", "0", "-f", "m4v"], 45.0);
    check_ffmpeg_stream("sp_4mv", 160, 128, 6, &["-c:v", "mpeg4", "-q:v", "3", "-bf", "0", "-flags", "+mv4", "-f", "m4v"], 45.0);
}

#[test]
fn decodes_ffmpeg_advanced_simple_bframes() {
    check_ffmpeg_stream("asp_bframes", 176, 144, 7, &["-c:v", "mpeg4", "-q:v", "3", "-bf", "2", "-f", "m4v"], 44.0);
}

#[test]
fn decodes_ffmpeg_advanced_simple_mpeg_quant() {
    check_ffmpeg_stream("asp_mpeg_quant", 160, 128, 6, &["-c:v", "mpeg4", "-q:v", "3", "-bf", "0", "-mpeg_quant", "1", "-f", "m4v"], 44.0);
}

#[test]
fn decodes_ffmpeg_advanced_simple_qpel() {
    check_ffmpeg_stream("asp_qpel", 160, 128, 6, &["-c:v", "mpeg4", "-q:v", "3", "-bf", "0", "-flags", "+qpel", "-f", "m4v"], 44.0);
}

#[test]
fn decodes_ffmpeg_h263_short_video_header() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("h263 short header oracle") else { return };
    let dir = craft_media_testkit::private_dir("mpeg4dec-h263");
    let path = dir.join("stream.h263");
    let enc_out = Command::new(&ff)
        .args([
            "-hide_banner",
            "-nostdin",
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=176x144:rate=25",
            "-frames:v",
            "1",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "h263",
            "-q:v",
            "4",
            "-f",
            "h263",
        ])
        .arg(&path)
        .output()
        .expect("run ffmpeg h263 encode");
    assert!(enc_out.status.success());

    let ref_out = Command::new(&ff)
        .args(["-hide_banner", "-nostdin", "-v", "error", "-f", "h263", "-i"])
        .arg(&path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .expect("run ffmpeg h263 decode");
    assert!(ref_out.status.success());

    let bytes = std::fs::read(&path).expect("read h263");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dec = Decoder::new();
    let pics = dec.decode(&bytes, 42).expect("decode h263 short header");
    assert_eq!(pics.len(), 1);
    assert_eq!((pics[0].width, pics[0].height), (176, 144));
    assert_eq!(pics[0].pts, 42);
    let p = psnr(&picture_bytes(&pics[0]), &ref_out.stdout);
    assert!(p >= 45.0, "H.263 short header PSNR {p:.2} dB");
}

fn _assert_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<Decoder>();
    check::<Picture>();
    let _ = Path::new("");
}
