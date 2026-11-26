//! The MPEG-4 Part 2 encoder against ffmpeg / ffprobe as external decoder oracles (skipped when
//! absent), plus API and robustness checks that need no oracle.

use std::path::{Path, PathBuf};
use std::process::Command;

use craft_video::mpeg4::{ColorConfig, Encoder, EncoderConfig, Error, Packet, RateControl, YuvFrame};

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

fn ffmpeg() -> Option<PathBuf> {
    craft_media_testkit::ffmpeg_or_skip("mpeg4 encoder oracle")
}

fn out_dir(name: &str) -> PathBuf {
    craft_media_testkit::fixtures_dir(&format!("mpeg4enc/{name}"))
}

fn write_stream(path: &Path, packets: &[Packet]) {
    let bytes: Vec<u8> = packets.iter().flat_map(|p| p.data.iter().copied()).collect();
    std::fs::write(path, bytes).expect("write stream");
}

/// ffmpeg's decode of a raw MPEG-4 Part 2 stream: (diagnostics, yuv420p frames).
fn ffmpeg_decode(ff: &Path, path: &Path) -> (String, Vec<u8>) {
    let out = Command::new(ff)
        .args(["-hide_banner", "-nostdin", "-v", "warning", "-f", "m4v", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
    (String::from_utf8_lossy(&out.stderr).into_owned(), out.stdout)
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let se: f64 = a.iter().zip(b).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum();
    if se == 0.0 {
        return 99.0;
    }
    10.0 * (255.0f64 * 255.0 * a.len() as f64 / se).log10()
}

fn planes(y: &Yuv) -> Vec<u8> {
    [&y.y[..], &y.u[..], &y.v[..]].concat()
}

/// An encoded test sequence: the packets, and for each frame the encoder's reconstruction and the
/// source (planes Y, U, V back to back).
struct Encoded {
    enc: Encoder,
    packets: Vec<Packet>,
    recon: Vec<Vec<u8>>,
    src: Vec<Vec<u8>>,
}

/// Encode `n` frames, keeping the encoder's reconstruction of each.
fn encode(cfg: EncoderConfig, n: usize) -> Encoded {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let mut enc = Encoder::new(cfg).expect("encoder");
    let (mut packets, mut recon, mut src) = (Vec::new(), Vec::new(), Vec::new());
    for t in 0..n {
        let pic = synth(w, h, t);
        packets.push(enc.encode(&pic.frame()).expect("encode"));
        let r = enc.reconstruction().expect("reconstruction");
        recon.push([r.y, r.u, r.v].concat());
        src.push(planes(&pic));
    }
    Encoded { enc, packets, recon, src }
}

/// ffmpeg decodes the stream without a diagnostic, frame for frame, to the encoder's own
/// reconstruction (within IDCT rounding) and close to the source.
fn check_against_ffmpeg(name: &str, cfg: EncoderConfig, n: usize, min_psnr: f64) {
    let Some(ff) = ffmpeg() else { return };
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let Encoded { packets, recon, src, .. } = encode(cfg, n);
    let path = out_dir(name).join("stream.m4v");
    write_stream(&path, &packets);
    let (diag, dec) = ffmpeg_decode(&ff, &path);
    assert!(diag.trim().is_empty(), "{name}: ffmpeg reported: {diag}");
    let size = w * h * 3 / 2;
    assert_eq!(dec.len(), size * n, "{name}: decoded frame count");
    for (i, frame) in dec.chunks(size).enumerate() {
        let drift = psnr(&recon[i], frame);
        assert!(drift >= 48.0, "{name}: frame {i} differs from the encoder's reconstruction ({drift:.1} dB)");
        let q = psnr(&src[i], frame);
        assert!(q >= min_psnr, "{name}: frame {i} at {q:.1} dB against the source (want {min_psnr})");
    }
}

#[test]
fn ffmpeg_decodes_constant_quantiser_streams() {
    for (w, h, q, min) in [(176, 144, 2, 38.0), (320, 240, 5, 31.0), (200, 120, 12, 26.0), (64, 48, 31, 20.0)] {
        let mut cfg = EncoderConfig::new(w, h, 25, 1);
        cfg.rate = RateControl::Quant(q);
        cfg.keyint = 12;
        check_against_ffmpeg(&format!("cq_{w}x{h}_q{q}"), cfg, 30, min);
    }
}

/// Sizes that are not a multiple of 16 leave partial macroblocks at the right and bottom edges.
#[test]
fn ffmpeg_decodes_partial_macroblocks() {
    for (w, h) in [(18, 34), (98, 66), (2, 2)] {
        let mut cfg = EncoderConfig::new(w, h, 30000, 1001);
        cfg.rate = RateControl::Quant(3);
        check_against_ffmpeg(&format!("edge_{w}x{h}"), cfg, 12, 30.0);
    }
}

/// More than 128 P-VOPs: every macroblock is refreshed with an intra coding before IDCT
/// mismatch can build up, and the time code crosses several seconds.
#[test]
fn long_gop_stays_in_step_with_the_decoder() {
    let mut cfg = EncoderConfig::new(96, 64, 10, 1);
    cfg.keyint = 1000;
    check_against_ffmpeg("long_gop", cfg, 150, 30.0);
}

#[test]
fn bitrate_mode_lands_near_the_target() {
    let kbps = 400;
    let mut cfg = EncoderConfig::new(320, 240, 25, 1);
    cfg.rate = RateControl::Bitrate { kbps };
    cfg.keyint = 50;
    let n = 100;
    let Encoded { packets, .. } = encode(cfg.clone(), n);
    let bits: usize = packets.iter().map(|p| p.data.len() * 8).sum();
    let got = bits as f64 / (n as f64 / 25.0) / 1000.0;
    assert!((got - f64::from(kbps)).abs() < 0.15 * f64::from(kbps), "{got:.0} kbps for a {kbps} kbps target");
    check_against_ffmpeg("abr", cfg, 40, 25.0);
}

#[test]
fn ffprobe_reads_the_configuration() {
    let Some(ff) = craft_media_testkit::ffprobe() else { return };
    let mut cfg = EncoderConfig::new(352, 288, 30000, 1001);
    cfg.sar = (12, 11);
    cfg.color = Some(ColorConfig { primaries: 6, transfer: 6, matrix: 6, full_range: false });
    let Encoded { packets, .. } = encode(cfg, 3);
    let path = out_dir("probe").join("stream.m4v");
    write_stream(&path, &packets);
    let out = Command::new(ff)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,profile,width,height,sample_aspect_ratio,color_primaries,color_space",
            "-of",
            "default=nw=1",
        ])
        .arg(&path)
        .output()
        .expect("run ffprobe");
    let text = String::from_utf8_lossy(&out.stdout);
    for want in ["codec_name=mpeg4", "width=352", "height=288", "sample_aspect_ratio=12:11", "color_primaries=smpte170m", "color_space=smpte170m"] {
        assert!(text.lines().any(|l| l == want), "ffprobe: missing {want} in\n{text}");
    }
    assert!(text.contains("Simple"), "ffprobe profile:\n{text}");
}

/// With the headers out of band, a decoder given the extradata (as MP4 or Matroska would)
/// decodes the VOPs.
#[test]
fn out_of_band_headers_are_the_extradata() {
    let Some(ff) = ffmpeg() else { return };
    let mut cfg = EncoderConfig::new(160, 96, 25, 1);
    cfg.headers_in_band = false;
    let Encoded { enc, packets, .. } = encode(cfg, 5);
    assert!(!packets[0].data.starts_with(&[0, 0, 1, 0xB0]));
    let mut bytes = enc.extradata().to_vec();
    bytes.extend(packets.iter().flat_map(|p| p.data.iter().copied()));
    let path = out_dir("oob").join("stream.m4v");
    std::fs::write(&path, bytes).expect("write");
    let (diag, dec) = ffmpeg_decode(&ff, &path);
    assert!(diag.trim().is_empty(), "{diag}");
    assert_eq!(dec.len(), 160 * 96 * 3 / 2 * 5);
    let p = enc.codec_params();
    assert_eq!(p.extradata, enc.extradata());
    assert_eq!(p.tag, Some(*b"mp4v"));
}

#[test]
fn keyframes_follow_keyint_and_requests() {
    let mut cfg = EncoderConfig::new(32, 32, 25, 1);
    cfg.keyint = 4;
    let mut enc = Encoder::new(cfg).expect("encoder");
    let pic = synth(32, 32, 0);
    let mut keys = Vec::new();
    for i in 0..10 {
        if i == 6 {
            enc.request_keyframe();
        }
        keys.push(enc.encode(&pic.frame()).expect("encode").keyframe);
    }
    assert_eq!(keys, [true, false, false, false, true, false, true, false, false, false]);
}

#[test]
fn bad_configurations_and_frames_are_errors() {
    let new = |w, h| Encoder::new(EncoderConfig::new(w, h, 25, 1)).err();
    assert_eq!(new(0, 16), Some(Error::InvalidDimensions(0, 16)));
    assert_eq!(new(17, 16), Some(Error::InvalidDimensions(17, 16)));
    assert_eq!(new(8192, 16), Some(Error::InvalidDimensions(8192, 16)));
    assert!(matches!(Encoder::new(EncoderConfig::new(16, 16, 0, 1)), Err(Error::InvalidFrameRate(0, 1))));
    assert!(matches!(Encoder::new(EncoderConfig::new(16, 16, 70000, 1)), Err(Error::InvalidFrameRate(..))));
    assert!(Encoder::new(EncoderConfig::new(16, 16, 140000, 2)).is_err());
    let mut cfg = EncoderConfig::new(16, 16, 25, 1);
    cfg.rate = RateControl::Quant(0);
    assert!(matches!(Encoder::new(cfg.clone()), Err(Error::InvalidConfig(_))));
    cfg.rate = RateControl::Quant(32);
    assert!(matches!(Encoder::new(cfg.clone()), Err(Error::InvalidConfig(_))));
    cfg.rate = RateControl::Quant(4);
    cfg.keyint = 0;
    assert!(matches!(Encoder::new(cfg), Err(Error::InvalidConfig(_))));

    let mut enc = Encoder::new(EncoderConfig::new(32, 16, 25, 1)).expect("encoder");
    let y = vec![0u8; 32 * 16];
    let c = vec![0u8; 16 * 8];
    assert_eq!(enc.encode(&YuvFrame { y: &y[1..], u: &c, v: &c, y_stride: 32, uv_stride: 16 }).err(), Some(Error::BadFrame));
    assert_eq!(enc.encode(&YuvFrame { y: &y, u: &c, v: &c, y_stride: 31, uv_stride: 16 }).err(), Some(Error::BadFrame));
    assert_eq!(enc.encode(&YuvFrame { y: &y, u: &c[..100], v: &c, y_stride: 32, uv_stride: 16 }).err(), Some(Error::BadFrame));
    assert_eq!(enc.encode(&YuvFrame { y: &y, u: &c, v: &c, y_stride: usize::MAX, uv_stride: 16 }).err(), Some(Error::BadFrame));
    // A failed frame leaves the encoder usable.
    assert!(enc.encode(&YuvFrame { y: &y, u: &c, v: &c, y_stride: 32, uv_stride: 16 }).is_ok());
}

/// Extreme pictures (flat black and white, full-range noise, hard alternation between them) at
/// the finest quantiser hit the largest levels and DC differences; ffmpeg still decodes them.
#[test]
fn extreme_content_at_the_finest_quantiser() {
    let (w, h) = (48, 32);
    let mut cfg = EncoderConfig::new(w as u32, h as u32, 25, 1);
    cfg.rate = RateControl::Quant(1);
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut packets = Vec::new();
    let mut recon = Vec::new();
    let mut seed = 1u32;
    for t in 0..8 {
        let y: Vec<u8> = (0..w * h)
            .map(|i| match t % 4 {
                0 => 0,
                1 => 255,
                2 => {
                    seed = hash(seed.wrapping_add(i as u32));
                    seed as u8
                }
                _ => {
                    if (i % w + i / w) % 2 == 0 {
                        0
                    } else {
                        255
                    }
                }
            })
            .collect();
        let c = vec![if t % 2 == 0 { 0 } else { 255 }; w * h / 4];
        packets.push(enc.encode(&YuvFrame { y: &y, u: &c, v: &c, y_stride: w, uv_stride: w / 2 }).expect("encode"));
        let r = enc.reconstruction().expect("reconstruction");
        recon.push([r.y, r.u, r.v].concat());
    }
    let Some(ff) = ffmpeg() else { return };
    let path = out_dir("extreme").join("stream.m4v");
    write_stream(&path, &packets);
    let (diag, dec) = ffmpeg_decode(&ff, &path);
    assert!(diag.trim().is_empty(), "{diag}");
    for (i, frame) in dec.chunks(w * h * 3 / 2).enumerate() {
        let d = psnr(&recon[i], frame);
        assert!(d >= 45.0, "frame {i}: {d:.1} dB from the reconstruction");
    }
}

/// The encoder's packets in an AVI (the track description through `VideoSpec::for_codec`) decode
/// in ffmpeg to exactly what the raw stream decodes to. Tagged `XVID` they would not: ffmpeg then
/// decodes with Xvid's IDCT.
#[test]
fn packets_play_from_an_avi_file() {
    use craft_media::avi::{AviWriter, MuxOptions, StreamSpec, VideoSpec};
    let Some(ff) = ffmpeg() else { return };
    let mut cfg = EncoderConfig::new(160, 96, 25, 1);
    cfg.keyint = 10;
    let Encoded { enc, packets, .. } = encode(cfg, 30);
    let spec = VideoSpec::for_codec(&enc.codec_params(), 25, 1).expect("an AVI FourCC for MPEG-4 Part 2");
    assert_eq!(spec.format.compression, *b"FMP4");
    let mut w = AviWriter::new(std::io::Cursor::new(Vec::new()), vec![StreamSpec::Video(spec)], MuxOptions::default()).expect("writer");
    for p in &packets {
        w.write_chunk(0, &p.data, p.keyframe).expect("chunk");
    }
    let avi = w.finish().expect("finish").into_inner();
    let dir = out_dir("avi");
    let (avi_path, es_path) = (dir.join("stream.avi"), dir.join("stream.m4v"));
    std::fs::write(&avi_path, avi).expect("write avi");
    write_stream(&es_path, &packets);
    let (_, want) = ffmpeg_decode(&ff, &es_path);
    let out = Command::new(&ff)
        .args(["-hide_banner", "-nostdin", "-v", "warning", "-i"])
        .arg(&avi_path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
        .output()
        .expect("run ffmpeg");
    let diag = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && diag.trim().is_empty(), "{diag}");
    assert_eq!(out.stdout.len(), 160 * 96 * 3 / 2 * 30);
    assert!(out.stdout == want, "the AVI decodes differently from the raw stream");
}
