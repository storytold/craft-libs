//! ffmpeg as an external decoder oracle (skipped when absent, see `craft_media_testkit`): every stream must
//! decode without an error to exactly the samples that went in, with the STREAMINFO MD5 matching
//! ffmpeg's hash of the decoded audio. From FilmCraft's `filmcraft-flac` tests.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::common;
use super::encoder::{Encoder, EncoderConfig, Error, STREAMINFO_OFFSET};

fn ffmpeg() -> Option<PathBuf> {
    common::ffmpeg()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

/// Test signals at `bps`, full scale: music-like (two sines and noise), a sweep, white noise,
/// a full-scale square, silence and a lone click.
fn signal(kind: &str, n: usize, bps: u8, seed: u64) -> Vec<i32> {
    let max = ((1i64 << (bps - 1)) - 1) as f64;
    let mut r = Rng(seed | 1);
    (0..n)
        .map(|i| {
            let t = i as f64 / 48_000.0;
            let v = match kind {
                "music" => {
                    0.5 * (2.0 * std::f64::consts::PI * 220.0 * t).sin() + 0.3 * (2.0 * std::f64::consts::PI * 1375.0 * t).sin() + 0.05 * r.next()
                }
                "sweep" => (2.0 * std::f64::consts::PI * (40.0 + 4000.0 * t) * t).sin() * 0.9,
                "noise" => r.next(),
                "square" => {
                    if (i / 37) % 2 == 0 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                "click" => f64::from(u8::from(i == n / 2)),
                _ => 0.0,
            };
            ((v * max).round() as i64).clamp(-(max as i64) - 1, max as i64) as i32
        })
        .collect()
}

fn encode(planar: &[Vec<i32>], cfg: EncoderConfig, chunk: usize) -> Result<Vec<u8>, Error> {
    let mut enc = Encoder::new(cfg)?;
    let mut file = enc.header(&[]);
    let n = planar.first().map_or(0, Vec::len);
    let mut at = 0;
    while at < n {
        let end = (at + chunk).min(n);
        let parts: Vec<&[i32]> = planar.iter().map(|c| &c[at..end]).collect();
        file.extend(enc.encode(&parts)?);
        at = end;
    }
    file.extend(enc.finish());
    let o = STREAMINFO_OFFSET as usize;
    file[o..o + 34].copy_from_slice(&enc.streaminfo());
    Ok(file)
}

fn run(ffmpeg: &Path, args: &[&str], input: &Path) -> (Vec<u8>, String) {
    let out = Command::new(ffmpeg).args(["-v", "error", "-i"]).arg(input).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
    (out.stdout, String::from_utf8_lossy(&out.stderr).into_owned())
}

/// Encode, decode with ffmpeg, compare every sample and the MD5; returns the file size.
fn round_trip(name: &str, planar: &[Vec<i32>], cfg: EncoderConfig, chunk: usize) -> Option<usize> {
    let ffmpeg = ffmpeg()?;
    let bps = cfg.bits_per_sample;
    let rate = cfg.sample_rate;
    let file = encode(planar, cfg, chunk).unwrap();
    let path = common::fixture_subdir("flac").join(format!("{name}.flac"));
    std::fs::write(&path, &file).unwrap();

    // samples: s32 holds each one shifted to the top bits
    let (raw, err) = run(&ffmpeg, &["-f", "s32le", "-c:a", "pcm_s32le", "-"], &path);
    assert!(err.is_empty(), "{name}: decoder errors: {err}");
    let decoded: Vec<i32> = raw.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b) >> (32 - bps)).collect();
    let n = planar[0].len();
    assert_eq!(decoded.len(), n * planar.len(), "{name}: sample count");
    for (i, frame) in decoded.chunks_exact(planar.len()).enumerate() {
        for (c, &v) in frame.iter().enumerate() {
            assert_eq!(v, planar[c][i], "{name}: channel {c} sample {i}");
        }
    }

    // STREAMINFO: total samples, rate, and the MD5 against ffmpeg's hash of the decoded audio. FLAC
    // hashes samples unscaled; ffmpeg rescales other sizes (20-bit becomes 24-bit), so only 8, 16,
    // 24 and 32 bit compare
    let si = &file[STREAMINFO_OFFSET as usize..STREAMINFO_OFFSET as usize + 34];
    let pcm = match bps {
        8 => Some("s8"),
        16 => Some("s16le"),
        24 => Some("s24le"),
        32 => Some("s32le"),
        _ => None,
    };
    if let Some(pcm) = pcm {
        let (hash, _) = run(&ffmpeg, &["-c:a", &format!("pcm_{pcm}"), "-f", "hash", "-hash", "md5", "-"], &path);
        let hash = String::from_utf8_lossy(&hash);
        let ours: String = si[18..34].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hash.trim().strip_prefix("MD5=").unwrap_or(""), ours, "{name}: MD5");
    }
    let total = (u64::from(si[13] & 0x0F) << 32) | u64::from(u32::from_be_bytes([si[14], si[15], si[16], si[17]]));
    assert_eq!(total, n as u64, "{name}: total samples");
    let si_rate = (u32::from(si[10]) << 12) | (u32::from(si[11]) << 4) | (u32::from(si[12]) >> 4);
    assert_eq!(si_rate, rate);
    Some(file.len())
}

#[test]
fn every_signal_round_trips_exactly() {
    for (kind, seed) in [("music", 1), ("sweep", 2), ("noise", 3), ("square", 4), ("silence", 5), ("click", 6)] {
        for bps in [16u8, 24, 32] {
            let planar = vec![signal(kind, 20_000, bps, seed), signal(kind, 20_000, bps, seed + 100)];
            if round_trip(&format!("{kind}-{bps}"), &planar, EncoderConfig::new(48_000, 2, bps), 4800).is_none() {
                return;
            }
        }
    }
}

/// 32-bit stereo at the extremes of i32: left minus right would need 33 bits, so the encoder must
/// not code a side channel (it overflowed `i32` and wrote garbage); every level decodes exactly.
#[test]
fn full_scale_32_bit_stereo_round_trips() {
    let n = 9_000;
    let left: Vec<i32> = (0..n).map(|i| if i % 3 == 0 { i32::MAX } else { i32::MIN }).collect();
    let right: Vec<i32> = left.iter().map(|&v| if v == i32::MAX { i32::MIN } else { i32::MAX }).collect();
    let planar = vec![left, right];
    for level in [0u8, 5, 8] {
        let cfg = EncoderConfig { level, ..EncoderConfig::new(48_000, 2, 32) };
        if round_trip(&format!("extremes-32-level{level}"), &planar, cfg, 4096).is_none() {
            return;
        }
    }
    let mut enc = Encoder::new(EncoderConfig::new(48_000, 2, 32)).unwrap();
    assert!(enc.encode(&[&[i32::MIN, i32::MAX], &[0, -1]]).is_ok());
}

/// Samples with unused low bits, on every kind of subframe and stereo mode: the decoder must put
/// the bits back before it undoes the stereo decorrelation.
#[test]
fn wasted_bits_round_trip() {
    for (bps, shift) in [(16u8, 3u32), (24, 8), (32, 16), (32, 8)] {
        let source = 16.min(bps - shift as u8);
        let pad = |kind, seed| -> Vec<i32> { signal(kind, 9_000, source, seed).iter().map(|&v| v << shift).collect() };
        for (kind, other) in [("music", "music"), ("noise", "noise"), ("square", "music"), ("click", "sweep")] {
            let planar = [pad(kind, 51), pad(other, 52)];
            if round_trip(&format!("wasted-{bps}-{shift}-{kind}"), &planar, EncoderConfig::new(48_000, 2, bps), 4096).is_none() {
                return;
            }
        }
    }
    // only the top bit can vary: one bit per sample remains
    let lowest: Vec<i32> = (0..9_000).map(|i| if i % 3 == 0 { -(1 << 23) } else { 0 }).collect();
    round_trip("wasted-to-one-bit", &[lowest], EncoderConfig::new(48_000, 1, 24), 4096);
}
