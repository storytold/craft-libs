//! The MPEG-2 encoder against ffmpeg as an external decoder: ffmpeg must decode our streams
//! without errors, to within the decoder suite's tolerance of the encoder's own reconstruction
//! (ffmpeg's integer IDCT differs from ours), and report the profile, level and picture types
//! we signal. Source pictures come from ffmpeg's `testsrc2`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use craft_video::mpeg2::{ChromaFormat, Encoder, EncoderConfig, RateControl, ReconFrame, YuvFrame};

use crate::common::{Stats, fixture_dir, pix_fmt_of};
use crate::oracle::{MAX_DIFFERING, MIN_PSNR, TOL_MAX};

/// Raw source frames from ffmpeg's `testsrc2`.
fn source(ff: &Path, w: u32, h: u32, frames: usize, pix_fmt: &str) -> Vec<u8> {
    let o = Command::new(ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size={w}x{h}:rate=25"),
            "-frames:v",
            &frames.to_string(),
        ])
        .args(["-pix_fmt", pix_fmt, "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .output()
        .expect("run ffmpeg");
    assert!(o.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}

fn sizes(cfg: &EncoderConfig) -> (usize, usize, usize) {
    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let ch = if cfg.chroma == ChromaFormat::Yuv422 { h } else { h / 2 };
    (w * h, w / 2 * ch, w / 2)
}

/// Encode `raw` frames; returns the elementary stream and the reconstruction.
fn encode(cfg: &EncoderConfig, raw: &[u8]) -> (Vec<u8>, Vec<ReconFrame>) {
    let (ys, cs, cw) = sizes(cfg);
    let fsize = ys + 2 * cs;
    let mut enc = Encoder::new(cfg.clone()).expect("config");
    enc.set_recon_capture(true);
    let mut es = Vec::new();
    for (i, f) in raw.chunks_exact(fsize).enumerate() {
        let frame = YuvFrame { y: &f[..ys], cb: &f[ys..ys + cs], cr: &f[ys + cs..], y_stride: cfg.width as usize, c_stride: cw };
        for p in enc.encode(&frame, i as i64).expect("encode") {
            es.extend_from_slice(&p.data);
        }
    }
    for p in enc.flush() {
        es.extend_from_slice(&p.data);
    }
    es.extend_from_slice(enc.end_of_sequence());
    (es, enc.take_recon())
}

/// ffmpeg's decode of `file`, failing on any decoding error.
fn ffmpeg_decode(ff: &Path, file: &Path, pix_fmt: &str) -> Vec<u8> {
    let o = Command::new(ff)
        .args(["-hide_banner", "-loglevel", "error", "-xerror", "-err_detect", "explode", "-i"])
        .arg(file)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", pix_fmt, "-"])
        .stdin(Stdio::null())
        .output()
        .expect("run ffmpeg");
    assert!(o.status.success() && o.stderr.is_empty(), "ffmpeg rejects {}: {}", file.display(), String::from_utf8_lossy(&o.stderr));
    o.stdout
}

/// ffprobe's stream description and per-frame picture types.
fn ffprobe(file: &Path) -> Option<(String, Vec<String>)> {
    let fp = craft_media_testkit::ffprobe()?;
    let stream = Command::new(&fp)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,profile,level,pix_fmt", "-of", "default=nw=1"])
        .arg(file)
        .output()
        .ok()?;
    let frames = Command::new(&fp)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "frame=pict_type", "-of", "csv=p=0"])
        .arg(file)
        .output()
        .ok()?;
    let types = String::from_utf8_lossy(&frames.stdout).lines().map(|l| l.trim_end_matches(',').to_string()).filter(|l| !l.is_empty()).collect();
    // "codec_name=…\nprofile=…" in ffprobe's order → "codec,profile,level,pix_fmt"
    let text = String::from_utf8_lossy(&stream.stdout).to_string();
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or("").to_string();
    Some((["codec_name", "profile", "level", "pix_fmt"].map(field).join(","), types))
}

fn write(name: &str, es: &[u8]) -> PathBuf {
    let path = fixture_dir().join(format!("enc_{name}.m2v"));
    std::fs::write(&path, es).expect("write stream");
    path
}

fn psnr_y(recon: &[ReconFrame], raw: &[u8], ys: usize, fsize: usize) -> f64 {
    let mut s = Stats::default();
    for (r, f) in recon.iter().zip(raw.chunks_exact(fsize)) {
        s.add(&r.y, &f[..ys]);
    }
    s.psnr()
}

struct Case {
    name: &'static str,
    cfg: EncoderConfig,
    frames: usize,
    /// ffprobe's "codec,profile,level,pix_fmt".
    probe: &'static str,
    /// Lowest luma PSNR against the source.
    min_psnr: f64,
}

fn check(ff: &Path, c: &Case) -> (Vec<u8>, f64) {
    let pix_fmt = pix_fmt_of(c.cfg.chroma);
    let raw = source(ff, c.cfg.width, c.cfg.height, c.frames, pix_fmt);
    let (ys, cs, _) = sizes(&c.cfg);
    let fsize = ys + 2 * cs;
    let (es, recon) = encode(&c.cfg, &raw);
    let path = write(c.name, &es);
    let decoded = ffmpeg_decode(ff, &path, pix_fmt);
    assert_eq!(decoded.len(), c.frames * fsize, "{}: ffmpeg's frame count", c.name);
    let mut all = Stats::default();
    for (i, (r, f)) in recon.iter().zip(decoded.chunks_exact(fsize)).enumerate() {
        let mut s = Stats::default();
        s.add(&r.y, &f[..ys]);
        s.add(&r.cb, &f[ys..ys + cs]);
        s.add(&r.cr, &f[ys + cs..]);
        assert!(s.max <= TOL_MAX && s.psnr() >= MIN_PSNR, "{} frame {i}: max {} psnr {:.2} against ffmpeg", c.name, s.max, s.psnr());
        all.merge(&s);
    }
    assert!((all.diff as f64) / (all.n as f64) <= MAX_DIFFERING, "{}: {} of {} samples differ from ffmpeg", c.name, all.diff, all.n);
    let quality = psnr_y(&recon, &raw, ys, fsize);
    assert!(quality >= c.min_psnr, "{}: luma PSNR {quality:.2} dB against the source", c.name);
    if let Some((stream, types)) = ffprobe(&path) {
        assert_eq!(stream, c.probe, "{}: ffprobe", c.name);
        let gop = c.cfg.gop_size as usize;
        let want: Vec<&str> = (0..c.frames).map(|i| if i % gop == 0 { "I" } else { "P" }).collect();
        assert_eq!(types, want, "{}: picture types", c.name);
    }
    let kbps = es.len() as f64 * 8.0 * 25.0 / c.frames as f64 / 1000.0;
    println!(
        "{}: {} frames, {kbps:.0} kbit/s, luma PSNR {quality:.2} dB, max diff to ffmpeg {}, {:.3}% differ",
        c.name,
        c.frames,
        all.max,
        100.0 * all.diff as f64 / all.n as f64
    );
    (es, kbps)
}

#[test]
fn ffmpeg_decodes_our_streams() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("mpeg2 encoder oracle") else { return };
    let cif = EncoderConfig { gop_size: 12, ..EncoderConfig::new(352, 288, 25, 1) };
    let cases = [
        Case { name: "cif_q4", cfg: cif.clone(), frames: 25, probe: "mpeg2video,Main,10,yuv420p", min_psnr: 36.0 },
        Case {
            name: "cif_intra",
            cfg: EncoderConfig { gop_size: 1, rate: RateControl::Quantiser(8), ..cif.clone() },
            frames: 6,
            probe: "mpeg2video,Main,10,yuv420p",
            min_psnr: 32.0,
        },
        Case {
            name: "cif_linear_coarse",
            cfg: EncoderConfig { nonlinear_quant: false, adaptive_quant: false, rate: RateControl::Quantiser(20), gop_size: 15, ..cif.clone() },
            frames: 20,
            probe: "mpeg2video,Main,10,yuv420p",
            min_psnr: 26.0,
        },
        Case {
            name: "sd_422",
            cfg: EncoderConfig { chroma: ChromaFormat::Yuv422, intra_dc_bits: 10, ..EncoderConfig::new(720, 576, 25, 1) },
            frames: 8,
            // 4:2:2@Main is indication 0x85; ffprobe prints its low four bits as the level
            probe: "mpeg2video,4:2:2,5,yuv422p",
            min_psnr: 36.0,
        },
    ];
    for c in &cases {
        check(&ff, c);
    }
}

#[test]
fn average_bitrate_reaches_its_target() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("mpeg2 encoder bitrate") else { return };
    for kbps in [800u32, 2500] {
        let c = Case {
            name: if kbps == 800 { "cif_abr_800" } else { "cif_abr_2500" },
            cfg: EncoderConfig { rate: RateControl::Abr { kbps }, gop_size: 12, ..EncoderConfig::new(352, 288, 25, 1) },
            frames: 72,
            probe: "mpeg2video,Main,10,yuv420p",
            min_psnr: 26.0,
        };
        let (_, got) = check(&ff, &c);
        let err = got / f64::from(kbps) - 1.0;
        assert!(err.abs() < 0.10, "{kbps} kbit/s target: got {got:.0} kbit/s ({:+.1}%)", 100.0 * err);
    }
}
