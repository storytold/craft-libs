//! The H.263 encoder: every picture it reports as reconstructed must be what the crate's decoder
//! and ffmpeg (an external oracle, skipped when absent) make of its packets.

use std::path::Path;
use std::process::Command;

use craft_video::h263::{Decoder, EncodeError, Encoder, EncoderConfig, Packet, RateControl, Reconstruction, YuvFrame};

struct Yuv {
    w: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Yuv {
    fn frame(&self) -> YuvFrame<'_> {
        YuvFrame { y: &self.y, u: &self.u, v: &self.v, y_stride: self.w, uv_stride: self.w / 2 }
    }
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// Picture `t` of a moving test sequence: a panning texture (sub-sample motion), a drifting
/// gradient, hard-edged blocks moving a whole number of samples a frame, and a little noise.
fn synth(w: usize, h: usize, t: usize) -> Yuv {
    let tf = t as f32;
    let mut y = vec![0u8; w * h];
    for yy in 0..h {
        for xx in 0..w {
            let (px, py) = (xx as f32 + tf * 1.5, yy as f32 + tf * 0.5);
            let tex = ((px / 9.0).sin() + (py / 7.0).cos() + ((px + py) / 23.0).sin()) * 35.0;
            let grad = 40.0 * ((xx as f32 / w as f32) * 4.0 + tf * 0.05).sin();
            let bx = (xx + 3 * t) / 24;
            let by = yy / 24;
            let block = if (bx + by).is_multiple_of(3) { 50.0 } else { -20.0 };
            let noise = (hash((xx as u32) ^ (yy as u32) << 12 ^ (t as u32) << 24) % 9) as f32 - 4.0;
            y[yy * w + xx] = (128.0 + tex + grad + block + noise).clamp(0.0, 255.0) as u8;
        }
    }
    let (cw, ch) = (w / 2, h / 2);
    let mut u = vec![0u8; cw * ch];
    let mut v = vec![0u8; cw * ch];
    for yy in 0..ch {
        for xx in 0..cw {
            let fx = xx as f32 / cw as f32;
            let fy = yy as f32 / ch as f32;
            u[yy * cw + xx] = (128.0 + 60.0 * (fx * 3.0 + tf * 0.03).sin()) as u8;
            v[yy * cw + xx] = (128.0 + 50.0 * (fy * 5.0 - tf * 0.04).cos() * fx) as u8;
        }
    }
    Yuv { w, y, u, v }
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "plane sizes differ");
    let sse: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    if sse == 0.0 { 99.0 } else { 10.0 * (255.0 * 255.0 * a.len() as f64 / sse).log10() }
}

fn planes(r: &Reconstruction) -> Vec<u8> {
    [&r.y[..], &r.u[..], &r.v[..]].concat()
}

/// Encode `frames` pictures of the test sequence.
fn encode(cfg: EncoderConfig, frames: usize) -> (Vec<Packet>, Vec<Reconstruction>) {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut packets = Vec::new();
    let mut recon = Vec::new();
    for t in 0..frames {
        let yuv = synth(w, h, t);
        packets.push(enc.encode(&yuv.frame()).expect("encode"));
        recon.push(enc.reconstruction().expect("reconstruction"));
    }
    (packets, recon)
}

/// The crate's decoder on every packet, compared with what the encoder reconstructed.
fn check_roundtrip(name: &str, cfg: EncoderConfig, frames: usize, min_psnr: f64) -> Vec<Packet> {
    let (w, h) = (cfg.width, cfg.height);
    let (packets, recon) = encode(cfg, frames);
    let mut dec = Decoder::new();
    for (i, (p, r)) in packets.iter().zip(&recon).enumerate() {
        let pics = dec.decode(&p.data, p.pts).unwrap_or_else(|e| panic!("{name}: picture {i}: {e}"));
        assert_eq!(pics.len(), 1, "{name}: picture {i}: one picture per packet");
        let pic = &pics[0];
        assert_eq!((pic.width, pic.height), (w, h), "{name}: size");
        let got = [&pic.y[..], &pic.u[..], &pic.v[..]].concat();
        let q = psnr(&got, &planes(r));
        assert!(q >= min_psnr, "{name}: picture {i} differs from the encoder's reconstruction ({q:.1} dB)");
    }
    packets
}

/// ffmpeg's decode of a raw H.263 stream, compared with the encoder's reconstruction.
fn check_oracle(name: &str, packets: &[Packet], recon: &[Reconstruction], min_psnr: f64) {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("h263 encoder oracle") else { return };
    let dir = craft_media_testkit::private_dir(&format!("h263enc-{name}"));
    let path = dir.join("stream.h263");
    std::fs::write(&path, packets.iter().flat_map(|p| p.data.iter().copied()).collect::<Vec<u8>>()).expect("write stream");
    let out = decode_with_ffmpeg(&ff, &path);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{name}: ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stderr.is_empty(), "{name}: ffmpeg reported: {}", String::from_utf8_lossy(&out.stderr));
    let frame_bytes = planes(&recon[0]).len();
    assert_eq!(out.stdout.len(), frame_bytes * recon.len(), "{name}: ffmpeg decoded {} bytes", out.stdout.len());
    for (i, (got, r)) in out.stdout.chunks_exact(frame_bytes).zip(recon).enumerate() {
        let q = psnr(got, &planes(r));
        assert!(q >= min_psnr, "{name}: picture {i}: ffmpeg's decode is {q:.1} dB from the encoder's reconstruction");
    }
}

fn decode_with_ffmpeg(ff: &Path, path: &Path) -> std::process::Output {
    Command::new(ff)
        .args(["-hide_banner", "-nostdin", "-v", "error", "-f", "h263", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .expect("run ffmpeg")
}

fn config(w: u32, h: u32) -> EncoderConfig {
    let mut cfg = EncoderConfig::new(w, h, 25, 1);
    cfg.rate = RateControl::Quant(6);
    cfg
}

#[test]
fn standard_sizes_decode_to_the_reconstruction() {
    for (name, w, h) in [("subqcif", 128, 96), ("qcif", 176, 144), ("cif", 352, 288)] {
        let (packets, recon) = encode(config(w, h), 8);
        check_roundtrip(name, config(w, h), 8, 45.0);
        check_oracle(name, &packets, &recon, 45.0);
    }
}

#[test]
fn custom_sizes_and_aspect_ratios_use_the_custom_picture_format() {
    for (name, w, h, sar) in
        [("odd-size", 200, 120, (12, 11)), ("narrow", 100, 68, (12, 11)), ("square-pixels", 176, 144, (1, 1)), ("extended-aspect", 176, 144, (7, 5))]
    {
        let mut cfg = config(w, h);
        cfg.sar = sar;
        let (packets, recon) = encode(cfg.clone(), 6);
        let decoded = check_roundtrip(name, cfg, 6, 45.0);
        assert_eq!(decoded[0].data, packets[0].data, "{name}: encoding is deterministic");
        check_oracle(name, &packets, &recon, 45.0);
    }
}

#[test]
fn the_decoder_reports_the_signalled_pixel_aspect_ratio() {
    let mut cfg = config(176, 144);
    cfg.sar = (7, 5);
    let (packets, _) = encode(cfg, 1);
    let mut dec = Decoder::new();
    dec.decode(&packets[0].data, 0).expect("decode");
    assert_eq!(dec.sequence().expect("sequence").pixel_aspect, (7, 5));
}

#[test]
fn gob_headers_resynchronise_and_decode_to_the_same_picture() {
    let mut cfg = config(176, 144);
    cfg.gob_headers = true;
    let (packets, recon) = encode(cfg.clone(), 8);
    check_roundtrip("gob-headers", cfg, 8, 45.0);
    check_oracle("gob-headers", &packets, &recon, 45.0);
    let plain = encode(config(176, 144), 8).0;
    assert!(packets[0].data.len() > plain[0].data.len(), "GOB headers take bits");
}

#[test]
fn tall_pictures_use_a_gob_of_several_rows() {
    let mut cfg = config(64, 432);
    cfg.gob_headers = true;
    let (packets, recon) = encode(cfg.clone(), 4);
    check_roundtrip("tall", cfg, 4, 45.0);
    check_oracle("tall", &packets, &recon, 45.0);
}

#[test]
fn inter_pictures_are_smaller_than_intra_ones() {
    let (packets, _) = encode(config(176, 144), 6);
    assert!(packets[0].keyframe && packets[1..].iter().all(|p| !p.keyframe));
    assert!(packets[1..].iter().all(|p| p.data.len() < packets[0].data.len()), "P-pictures exploit the motion");
}

#[test]
fn quality_follows_the_quantiser() {
    let src = synth(176, 144, 0);
    let quality = |q: u8| {
        let mut cfg = config(176, 144);
        cfg.rate = RateControl::Quant(q);
        let mut enc = Encoder::new(cfg).expect("encoder");
        let p = enc.encode(&src.frame()).expect("encode");
        let r = enc.reconstruction().expect("reconstruction");
        (psnr(&r.y, &src.y), p.data.len())
    };
    let (fine, fine_bytes) = quality(2);
    let (coarse, coarse_bytes) = quality(24);
    assert!(fine > coarse + 6.0, "{fine:.1} dB against {coarse:.1} dB");
    assert!(fine_bytes > 2 * coarse_bytes);
}

#[test]
fn keyframes_come_when_asked_and_at_the_interval() {
    let mut cfg = config(176, 144);
    cfg.keyint = 3;
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut keys = Vec::new();
    for t in 0..7 {
        if t == 4 {
            enc.request_keyframe();
        }
        keys.push(enc.encode(&synth(176, 144, t).frame()).expect("encode").keyframe);
    }
    assert_eq!(keys, [true, false, false, true, true, false, false]);
}

#[test]
fn average_bitrate_follows_the_target() {
    let kbps = 300;
    let mut cfg = config(176, 144);
    cfg.rate = RateControl::Bitrate { kbps };
    let frames = 50;
    let (packets, _) = encode(cfg, frames);
    let bits: usize = packets.iter().map(|p| p.data.len() * 8).sum();
    let rate = bits as f64 / frames as f64 * 25.0 / 1000.0;
    assert!(rate > f64::from(kbps) * 0.5 && rate < f64::from(kbps) * 1.6, "{rate:.0} kbps for a {kbps} kbps target");
}

#[test]
fn a_long_sequence_does_not_drift() {
    // Every macroblock is refreshed within 132 inter codings, so a decoder's own inverse
    // transform cannot pull its pictures away from the encoder's.
    let (packets, recon) = encode(config(64, 48), 140);
    let mut dec = Decoder::new();
    let last = packets.len() - 1;
    for (i, p) in packets.iter().enumerate() {
        let pics = dec.decode(&p.data, p.pts).expect("decode");
        if i == last {
            let got = [&pics[0].y[..], &pics[0].u[..], &pics[0].v[..]].concat();
            assert!(psnr(&got, &planes(&recon[i])) > 40.0);
        }
    }
}

#[test]
fn bad_configurations_and_frames_are_refused() {
    assert!(matches!(Encoder::new(config(0, 144)), Err(EncodeError::InvalidDimensions(0, 144))));
    assert!(matches!(Encoder::new(config(178, 144)), Err(EncodeError::InvalidDimensions(178, 144))));
    assert!(matches!(Encoder::new(config(2052, 144)), Err(EncodeError::InvalidDimensions(..))));
    assert!(matches!(Encoder::new(EncoderConfig::new(176, 144, 0, 1)), Err(EncodeError::InvalidFrameRate(0, 1))));
    let mut cfg = config(176, 144);
    cfg.rate = RateControl::Quant(0);
    assert!(matches!(Encoder::new(cfg), Err(EncodeError::InvalidConfig(_))));
    let mut cfg = config(176, 144);
    cfg.keyint = 0;
    assert!(matches!(Encoder::new(cfg), Err(EncodeError::InvalidConfig(_))));
    let mut cfg = config(176, 144);
    cfg.sar = (0, 1);
    assert!(matches!(Encoder::new(cfg), Err(EncodeError::InvalidConfig(_))));

    let mut enc = Encoder::new(config(176, 144)).expect("encoder");
    let short = YuvFrame { y: &[0; 100], u: &[0; 10], v: &[0; 10], y_stride: 176, uv_stride: 88 };
    assert!(matches!(enc.encode(&short), Err(EncodeError::BadFrame)));
}

#[test]
fn flat_and_extreme_pictures_survive_the_forbidden_dc_values() {
    // INTRADC of 0 and 128 are forbidden, and a flat mid-grey picture sits on 128.
    for level in [0u8, 16, 128, 255] {
        let (w, h) = (176usize, 144usize);
        let (y, u, v) = (vec![level; w * h], vec![level; w * h / 4], vec![level; w * h / 4]);
        let frame = YuvFrame { y: &y, u: &u, v: &v, y_stride: w, uv_stride: w / 2 };
        let mut enc = Encoder::new(config(176, 144)).expect("encoder");
        let p = enc.encode(&frame).expect("encode");
        let pics = Decoder::new().decode(&p.data, 0).expect("decode");
        let mean = pics[0].y.iter().map(|&v| u32::from(v)).sum::<u32>() / pics[0].y.len() as u32;
        assert!(mean.abs_diff(u32::from(level)) <= 2, "level {level} decoded to {mean}");
    }
}
