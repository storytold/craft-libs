//! `craft_audio::decode` routes each container and codec to the home-grown demuxers and
//! decoders: every pair decodes, sample-aligned, to its presentation length (edit lists, Matroska
//! `CodecDelay` and `DiscardPadding`, Ogg Opus pre-skip and end trimming applied), within an SNR
//! of ffmpeg's decode; truncated and corrupted copies never panic. Fixtures are written by ffmpeg
//! (skipped without it).

use craft_audio::{FileFormat, decode, probe};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Five seconds of two tones and a little noise per channel.
const STEREO: &str =
    "aevalsrc=0.4*sin(2*PI*220*t)+0.15*sin(2*PI*1375*t)+0.05*(random(0)-0.5)|0.35*sin(2*PI*330*t)+0.2*sin(2*PI*2750*t*(1+t/20))+0.05*(random(1)-0.5)";
const SURROUND_EXTRA: &str = "|0.3*sin(2*PI*440*t)|0.1*sin(2*PI*60*t)|0.2*sin(2*PI*880*t)|0.2*sin(2*PI*990*t)";
const MONO: &str = "aevalsrc=0.4*sin(2*PI*220*t)";

struct Case {
    name: &'static str,
    surround: bool,
    mono: bool,
    rate: u32,
    args: &'static [&'static str],
    format: FileFormat,
    channels: usize,
    /// Presented frames, or `None` for a raw stream (as many as ffmpeg decodes).
    frames: Option<usize>,
    /// Minimum SNR against ffmpeg (AC-3 is limited by decoder-specific dither, §7.3.4).
    min_snr: f64,
}

const CASES: &[Case] = &[
    Case {
        name: "aac.aac",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "aac", "-b:a", "128k", "-aac_pns", "0"],
        format: FileFormat::Aac,
        channels: 2,
        frames: None,
        min_snr: 120.0,
    },
    Case {
        name: "aac.m4a",
        surround: false,
        mono: false,
        rate: 44_100,
        args: &["-c:a", "aac", "-b:a", "128k", "-aac_pns", "0"],
        format: FileFormat::Aac,
        channels: 2,
        frames: Some(220_500),
        min_snr: 120.0,
    },
    Case {
        name: "aac-5.1.m4a",
        surround: true,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "aac", "-b:a", "384k", "-aac_pns", "0"],
        format: FileFormat::Aac,
        channels: 6,
        frames: Some(240_000),
        min_snr: 120.0,
    },
    Case {
        name: "aac.mkv",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "aac", "-b:a", "128k", "-aac_pns", "0"],
        format: FileFormat::Aac,
        channels: 2,
        frames: Some(240_000),
        min_snr: 120.0,
    },
    Case {
        name: "mp2.mp2",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "mp2"],
        format: FileFormat::Mp3,
        channels: 2,
        frames: None,
        min_snr: 80.0,
    },
    // ffmpeg's default for mono MP2 is 384 kbit/s, which ISO/IEC 11172-3 does not allow.
    Case {
        name: "mp2-mono.mp2",
        surround: false,
        mono: true,
        rate: 48_000,
        args: &["-c:a", "mp2"],
        format: FileFormat::Mp3,
        channels: 1,
        frames: None,
        min_snr: 80.0,
    },
    Case {
        name: "mp2.mkv",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "mp2"],
        format: FileFormat::Mp3,
        channels: 2,
        frames: Some(240_000),
        min_snr: 80.0,
    },
    Case {
        name: "mp2.mp4",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "mp2"],
        format: FileFormat::Mp3,
        channels: 2,
        frames: Some(240_000),
        min_snr: 80.0,
    },
    Case {
        name: "ac3.ac3",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "ac3", "-b:a", "192k"],
        format: FileFormat::Other,
        channels: 2,
        frames: None,
        min_snr: 25.0,
    },
    Case {
        name: "ac3-5.1.ac3",
        surround: true,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "ac3", "-b:a", "448k"],
        format: FileFormat::Other,
        channels: 6,
        frames: None,
        min_snr: 25.0,
    },
    Case {
        name: "eac3.eac3",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "eac3", "-b:a", "192k"],
        format: FileFormat::Other,
        channels: 2,
        frames: None,
        min_snr: 25.0,
    },
    Case {
        name: "ac3.mp4",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "ac3"],
        format: FileFormat::Other,
        channels: 2,
        frames: Some(240_000),
        min_snr: 25.0,
    },
    Case {
        name: "eac3-5.1.mkv",
        surround: true,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "eac3"],
        format: FileFormat::Other,
        channels: 6,
        frames: Some(240_000),
        min_snr: 25.0,
    },
    Case {
        name: "opus.opus",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "libopus", "-b:a", "128k"],
        format: FileFormat::Ogg,
        channels: 2,
        frames: Some(240_000),
        min_snr: 120.0,
    },
    Case {
        name: "opus-5.1.opus",
        surround: true,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "libopus", "-b:a", "256k", "-mapping_family", "1"],
        format: FileFormat::Ogg,
        channels: 6,
        frames: Some(240_000),
        min_snr: 120.0,
    },
    Case {
        name: "opus.webm",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "libopus", "-b:a", "96k"],
        format: FileFormat::Other,
        channels: 2,
        frames: Some(240_000),
        min_snr: 120.0,
    },
    Case {
        name: "opus.mp4",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "libopus", "-b:a", "96k"],
        format: FileFormat::Other,
        channels: 2,
        frames: Some(240_000),
        min_snr: 120.0,
    },
    Case {
        name: "pcm-s24be.mov",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "pcm_s24be"],
        format: FileFormat::Other,
        channels: 2,
        frames: Some(240_000),
        min_snr: 140.0,
    },
    Case {
        name: "pcm-f32.mkv",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "pcm_f32le"],
        format: FileFormat::Other,
        channels: 2,
        frames: Some(240_000),
        min_snr: 140.0,
    },
];

fn run(ff: &Path, args: &[&str]) -> bool {
    Command::new(ff).args(["-v", "error", "-y"]).args(args).output().is_ok_and(|o| o.status.success())
}

/// The fixture for `c`, written by ffmpeg (cached), or `None` when this ffmpeg lacks an encoder.
fn fixture(ff: &Path, c: &Case) -> Option<PathBuf> {
    let out = crate::testkit::fixtures_dir("routing").join(c.name);
    let source = match (c.mono, c.surround) {
        (true, _) => MONO.to_string(),
        (_, true) => format!("{STEREO}{SURROUND_EXTRA}"),
        _ => STEREO.to_string(),
    };
    let input = format!("{source}:s={}", c.rate);
    crate::testkit::fixtures::generate(&out, |tmp| {
        let tmp = tmp.to_string_lossy();
        let mut args = vec!["-f", "lavfi", "-i", &input, "-t", "5"];
        args.extend_from_slice(c.args);
        args.push(&tmp);
        run(ff, &args)
    })
}

/// ffmpeg's decode of `file` as interleaved f32.
fn reference(ff: &Path, file: &Path) -> Vec<f32> {
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-f", "f32le", "-c:a", "pcm_f32le", "-"]).output().unwrap();
    assert!(out.status.success(), "ffmpeg decode of {}", file.display());
    out.stdout.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// SNR (dB) of `ours` (planar) against `reference` (interleaved), sample-aligned, over the
/// frames both hold.
fn snr(ours: &[Vec<f32>], reference: &[f32]) -> f64 {
    let ch = ours.len();
    let n = ours.iter().map(Vec::len).min().unwrap_or(0).min(reference.len() / ch);
    let (mut s, mut e) = (0f64, 0f64);
    for i in 0..n {
        for (c, plane) in ours.iter().enumerate() {
            let r = f64::from(reference[i * ch + c]);
            let d = f64::from(plane[i]) - r;
            s += r * r;
            e += d * d;
        }
    }
    if e == 0.0 { f64::INFINITY } else { 10.0 * (s / e).log10() }
}

#[test]
fn every_container_and_codec_decodes_home_grown() {
    let ff = require_ffmpeg!();
    for c in CASES {
        let Some(path) = fixture(&ff, c) else {
            eprintln!("SKIPPED {}: this ffmpeg cannot write it", c.name);
            continue;
        };
        let bytes = std::fs::read(&path).unwrap();
        let (info, buf) = decode(&bytes, None).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let reference = reference(&ff, &path);
        assert_eq!((info.format, info.sample_rate, buf.channels.len()), (c.format, c.rate, c.channels), "{}", c.name);
        let expected = c.frames.unwrap_or(reference.len() / c.channels);
        assert_eq!(buf.frames(), expected, "{}: presented frames", c.name);
        let s = snr(&buf.channels, &reference);
        assert!(s >= c.min_snr, "{}: SNR {s:.1} dB < {}", c.name, c.min_snr);
        let p = probe(&bytes, None).unwrap();
        assert_eq!((p.format, p.sample_rate, usize::from(p.channels)), (c.format, c.rate, c.channels), "{}: probe", c.name);
    }
}

#[test]
fn truncated_and_corrupted_files_never_panic() {
    let ff = require_ffmpeg!();
    for c in CASES {
        let Some(path) = fixture(&ff, c) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        for cut in [1, 7, 64, 500, bytes.len() / 3, bytes.len() / 2, bytes.len() - 1] {
            let _ = decode(bytes.get(..cut).unwrap_or_default(), None);
            let _ = probe(bytes.get(..cut).unwrap_or_default(), None);
        }
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..6 {
            let mut m = bytes.clone();
            for _ in 0..16 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = (seed % m.len() as u64) as usize;
                m[i] ^= (seed >> 32) as u8 | 1;
            }
            let _ = decode(&m, None);
        }
    }
}
