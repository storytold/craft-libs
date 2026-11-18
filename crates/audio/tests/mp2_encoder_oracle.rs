//! The Layer II encoder (feature `mpa-encode`): every frame has the header and size its settings
//! give and decodes with our decoder; ffmpeg as an external decoder (skipped without ffmpeg, a
//! failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`) decodes the stream without a complaint, to the
//! right length, and its output, aligned by [`DELAY`], matches the input within a signal-to-noise
//! ratio per bit rate. The bounds sit well under what the encoder reaches today; white noise loses
//! the subbands above the table's last one (a sixth of it at 48 kHz), so its bound is low.

use std::path::Path;
use std::process::{Command, Stdio};

use craft_audio::mpa::encoder::{Config, DELAY, EncodeError, Encoder, bitrates, nearest_bitrate};
use craft_audio::mpa::{Decoder, Mode, Version, parse_header};

fn chords(rate: u32, n: usize, detune: f32) -> Vec<f32> {
    let f = [220.0, 277.18, 329.63, 440.0, 1318.5, 3520.0];
    let a = [0.2, 0.15, 0.12, 0.1, 0.05, 0.02];
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            f.iter().zip(a).map(|(f, a)| a * (2.0 * std::f32::consts::PI * f * (1.0 + detune) * t).sin()).sum::<f32>()
        })
        .collect()
}

fn noise(n: usize, amp: f32, seed: u64) -> Vec<f32> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

fn encode(cfg: Config, input: &[Vec<f32>], chunk: usize) -> Vec<u8> {
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut out = Vec::new();
    let n = input[0].len();
    let mut pos = 0;
    while pos < n {
        let end = (pos + chunk).min(n);
        let planes: Vec<&[f32]> = input.iter().map(|c| &c[pos..end]).collect();
        out.extend(enc.encode(&planes));
        pos = end;
    }
    out.extend(enc.flush());
    out
}

/// Split a stream into frames by its headers; every header must be one the settings give.
fn frames<'a>(bytes: &'a [u8], cfg: &Config) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let h = parse_header(&bytes[pos..]).unwrap_or_else(|e| panic!("frame {} at byte {pos}: {e}", out.len()));
        assert_eq!(h.layer, 2);
        assert_eq!(h.version, if cfg.sample_rate >= 32_000 { Version::Mpeg1 } else { Version::Mpeg2 });
        assert_eq!((h.sample_rate, h.bitrate_kbps), (cfg.sample_rate, cfg.bitrate_kbps));
        assert_eq!(h.mode, if cfg.channels == 1 { Mode::Mono } else { Mode::Stereo });
        assert!(!h.crc);
        out.push(&bytes[pos..pos + h.frame_bytes]);
        pos += h.frame_bytes;
    }
    assert_eq!(pos, bytes.len(), "the stream ends on a frame boundary");
    out
}

fn snr(input: &[f32], dec: &[f32]) -> f64 {
    let (mut s, mut n) = (0f64, 0f64);
    for (a, b) in input.iter().zip(dec) {
        s += f64::from(*a) * f64::from(*a);
        n += f64::from(a - b) * f64::from(a - b);
    }
    10.0 * (s / n.max(1e-30)).log10()
}

struct Case {
    name: &'static str,
    rate: u32,
    channels: usize,
    kbps: u32,
    signal: fn(u32, usize, usize) -> Vec<f32>,
    min_snr: f64,
}

fn sig_chords(rate: u32, n: usize, c: usize) -> Vec<f32> {
    chords(rate, n, 0.003 * c as f32)
}
fn sig_noise(_rate: u32, n: usize, c: usize) -> Vec<f32> {
    noise(n, 0.25, 11 + c as u64)
}

const CASES: &[Case] = &[
    Case { name: "chords", rate: 48_000, channels: 2, kbps: 192, signal: sig_chords, min_snr: 30.0 },
    Case { name: "chords", rate: 44_100, channels: 2, kbps: 128, signal: sig_chords, min_snr: 25.0 },
    Case { name: "chords", rate: 44_100, channels: 1, kbps: 64, signal: sig_chords, min_snr: 25.0 },
    Case { name: "chords", rate: 32_000, channels: 2, kbps: 96, signal: sig_chords, min_snr: 20.0 },
    Case { name: "chords", rate: 22_050, channels: 2, kbps: 64, signal: sig_chords, min_snr: 20.0 },
    Case { name: "chords", rate: 24_000, channels: 1, kbps: 32, signal: sig_chords, min_snr: 18.0 },
    Case { name: "noise", rate: 48_000, channels: 2, kbps: 384, signal: sig_noise, min_snr: 4.0 },
    Case { name: "noise", rate: 44_100, channels: 2, kbps: 256, signal: sig_noise, min_snr: 4.0 },
];

fn inputs(c: &Case, n: usize) -> Vec<Vec<f32>> {
    (0..c.channels).map(|ch| (c.signal)(c.rate, n, ch)).collect()
}

#[test]
fn frames_have_the_size_and_header_their_settings_give_and_decode_with_our_decoder() {
    for c in CASES {
        let n = c.rate as usize * 2 + 777;
        let cfg = Config::cbr(c.rate, c.channels, c.kbps);
        let bytes = encode(cfg, &inputs(c, n), 1000);
        let frames = frames(&bytes, &cfg);
        // every input sample and the filter delay fit in the frames, and no more than one frame more
        let covered = frames.len() * 1152;
        assert!(covered >= n + DELAY as usize && covered < n + DELAY as usize + 1152, "{}: {} frames for {n} samples", c.name, frames.len());
        // 144 kbit/s-slots per frame, the padding slots adding up to the exact rate
        let nominal = f64::from(c.kbps) * 125.0 * covered as f64 / f64::from(c.rate);
        assert!((bytes.len() as f64 - nominal).abs() < 1.0 + f64::from(c.rate) / 1000.0, "{}: {} bytes against {nominal}", c.name, bytes.len());
        let mut dec = Decoder::new();
        let mut out = vec![Vec::new(); c.channels];
        for f in frames {
            let frame = dec.decode(f).unwrap_or_else(|e| panic!("{}: {e}", c.name));
            for (o, ch) in out.iter_mut().zip(frame.channels) {
                o.extend(ch);
            }
        }
        let d = DELAY as usize;
        for (ch, input) in inputs(c, n).iter().enumerate() {
            let s = snr(input, &out[ch][d..d + n]);
            println!("{:<7} {:>5} Hz {}ch {:>3} kbps: SNR {s:.2} dB (our decoder)", c.name, c.rate, c.channels, c.kbps);
            assert!(s >= c.min_snr, "{} {} Hz {}ch {} kbps: SNR {s:.2} dB under {}", c.name, c.rate, c.channels, c.kbps, c.min_snr);
        }
    }
}

#[test]
fn the_stream_does_not_depend_on_how_the_input_is_chunked() {
    let c = &CASES[1];
    let n = 44_100 + 333;
    let cfg = Config::cbr(c.rate, c.channels, c.kbps);
    let input = inputs(c, n);
    let whole = encode(cfg, &input, n);
    for chunk in [1, 7, 1152, 5000] {
        assert_eq!(encode(cfg, &input, chunk), whole, "chunk {chunk}");
    }
}

#[test]
fn silence_and_nothing_encode_to_valid_frames() {
    let cfg = Config::cbr(48_000, 2, 128);
    let silent = vec![vec![0f32; 5000]; 2];
    let bytes = encode(cfg, &silent, 5000);
    let fs = frames(&bytes, &cfg);
    let mut dec = Decoder::new();
    for f in fs {
        let frame = dec.decode(f).expect("decode");
        assert!(frame.channels.iter().flatten().all(|v| v.abs() < 1e-6));
    }
    assert!(Encoder::new(cfg).expect("encoder").flush().is_empty(), "no input, no frames");
}

#[test]
fn hostile_samples_are_tamed() {
    let cfg = Config::cbr(44_100, 2, 192);
    let mut x = vec![0f32; 4000];
    for (i, v) in x.iter_mut().enumerate() {
        *v = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30, 3.0, 0.5][i % 7];
    }
    let bytes = encode(cfg, &[x.clone(), x], 4000);
    let mut dec = Decoder::new();
    for f in frames(&bytes, &cfg) {
        dec.decode(f).expect("decode");
    }
}

#[test]
fn settings_that_cannot_be_carried_are_refused() {
    assert_eq!(Encoder::new(Config::cbr(44_000, 2, 128)).err(), Some(EncodeError::SampleRate(44_000)));
    assert_eq!(Encoder::new(Config::cbr(44_100, 3, 128)).err(), Some(EncodeError::Channels(3)));
    assert_eq!(Encoder::new(Config::cbr(44_100, 0, 128)).err(), Some(EncodeError::Channels(0)));
    // ISO/IEC 11172-3 allows neither 32 kbit/s for two channels nor 224 for one
    assert!(matches!(Encoder::new(Config::cbr(44_100, 2, 32)), Err(EncodeError::Bitrate { .. })));
    assert!(matches!(Encoder::new(Config::cbr(44_100, 1, 224)), Err(EncodeError::Bitrate { .. })));
    assert!(matches!(Encoder::new(Config::cbr(44_100, 2, 130)), Err(EncodeError::Bitrate { .. })));
    assert_eq!(bitrates(44_100, 1).len(), 10);
    assert_eq!(bitrates(22_050, 2).first(), Some(&8));
    assert!(bitrates(8_000, 2).is_empty());
    assert_eq!(nearest_bitrate(44_100, 2, 100), Some(96));
    assert_eq!(nearest_bitrate(44_100, 2, 1), Some(64));
    assert_eq!(nearest_bitrate(8_000, 2, 100), None);
}

fn ffmpeg_decode(ff: &Path, path: &Path, channels: usize) -> (Vec<Vec<f32>>, String) {
    let out = Command::new(ff)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .stdin(Stdio::null())
        .output()
        .expect("run ffmpeg");
    let samples: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    let planar = (0..channels).map(|c| samples.iter().skip(c).step_by(channels).copied().collect()).collect();
    (planar, String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn ffmpeg_decodes_our_streams_to_the_input() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("mp2enc");
    for c in CASES {
        let n = c.rate as usize * 2 + 777;
        let input = inputs(c, n);
        let bytes = encode(Config::cbr(c.rate, c.channels, c.kbps), &input, 1000);
        let path = dir.join(format!("{}_{}_{}ch_{}k.mp2", c.name, c.rate, c.channels, c.kbps));
        std::fs::write(&path, &bytes).expect("write");
        let (dec, err) = ffmpeg_decode(&ff, &path, c.channels);
        assert!(err.trim().is_empty(), "{}: ffmpeg reported: {err}", path.display());
        let d = DELAY as usize;
        assert!(dec[0].len() >= n + d, "{}: decoded {} samples, need {}", c.name, dec[0].len(), n + d);
        let s = (0..c.channels).map(|ch| snr(&input[ch], &dec[ch][d..d + n])).sum::<f64>() / c.channels as f64;
        println!("{:<7} {:>5} Hz {}ch {:>3} kbps: SNR {s:.2} dB (ffmpeg)", c.name, c.rate, c.channels, c.kbps);
        assert!(s >= c.min_snr, "{} {} Hz {}ch {} kbps: SNR {s:.2} dB under {}", c.name, c.rate, c.channels, c.kbps, c.min_snr);
    }
}
