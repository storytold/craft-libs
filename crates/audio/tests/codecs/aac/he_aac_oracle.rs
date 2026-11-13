//! HE-AAC (SBR) decoding against ffmpeg's decoder, on streams from ffmpeg's AudioToolbox encoder
//! (oracle only; skipped without ffmpeg or without `aac_at`, which only macOS builds have).

use std::process::Command;

use craft_audio::aac::{Decoder, split_adts};

use super::common::*;

/// ffmpeg, if it has the AudioToolbox AAC encoder.
fn ffmpeg_with_aac_at() -> Option<std::path::PathBuf> {
    let ff = ffmpeg()?;
    let out = Command::new(&ff).args(["-hide_banner", "-encoders"]).output().ok()?;
    if !String::from_utf8_lossy(&out.stdout).contains("aac_at") {
        eprintln!("SKIPPED: ffmpeg has no aac_at encoder");
        return None;
    }
    Some(ff)
}

fn input(rate: u32, channels: usize, secs: f32) -> Vec<Vec<f32>> {
    let n = (rate as f32 * secs) as usize;
    (0..channels)
        .map(|c| {
            let a = chords(rate, n, 0.004 * c as f32);
            let s = speech(rate, n, 11 + c as u64);
            let z = noise(n, 0.05, 3 + c as u64);
            let k = clicks(rate, n);
            a.iter().zip(&s).zip(&z).zip(&k).map(|(((a, s), z), k)| 0.4 * a + 0.4 * s * if c == 1 { -0.7 } else { 1.0 } + z + 0.3 * k).collect()
        })
        .collect()
}

/// ffmpeg's decode of a stream of `channels` channels. ffmpeg decodes mono HE-AAC as stereo (it
/// allows for parametric stereo the stream may hold), with the same signal in both channels when
/// there is none: of those, the first is taken (a downmix would scale it by -3 dB).
fn ffmpeg_decode_as(ff: &std::path::Path, file: &std::path::Path, channels: usize) -> Vec<Vec<f32>> {
    let mut planar = ffmpeg_decode_raw(ff, file, channels.max(2));
    planar.truncate(channels);
    planar
}

fn ffmpeg_decode_raw(ff: &std::path::Path, file: &std::path::Path, channels: usize) -> Vec<Vec<f32>> {
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-ac", &channels.to_string(), "-f", "f32le", "-"]).output().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    let samples: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect();
    (0..channels).map(|c| samples.iter().skip(c).step_by(channels).copied().collect()).collect()
}

/// Our decode of an ADTS stream, SBR on.
fn decode_sbr(data: &[u8]) -> (Vec<Vec<f32>>, u32) {
    let frames = split_adts(data).unwrap();
    let mut dec = Decoder::from_adts(&frames[0].0).unwrap();
    dec.set_sbr(true);
    let mut out: Vec<Vec<f32>> = Vec::new();
    for (_, au) in frames {
        let pcm = dec.decode(au).unwrap();
        out.resize(pcm.len(), Vec::new());
        for (o, p) in out.iter_mut().zip(pcm) {
            o.extend(p);
        }
    }
    (out, dec.sample_rate())
}

/// The lag of `b` behind `a` (within ±`range`) that correlates best.
fn best_lag(a: &[f32], b: &[f32], range: i64) -> i64 {
    let n = a.len().min(b.len()) as i64;
    let (from, to) = (range, n - range);
    (-range..=range)
        .max_by(|&x, &y| {
            let c = |lag: i64| (from..to).step_by(3).map(|i| a[i as usize] as f64 * b[(i + lag) as usize] as f64).sum::<f64>();
            c(x).total_cmp(&c(y))
        })
        .unwrap()
}

/// Energy in dB of `x[from..from + 2048]` within `lo..hi` Hz (Hann-windowed DFT).
fn band_db(x: &[f32], from: usize, rate: f32, lo: f32, hi: f32) -> f64 {
    let n = 2048;
    let seg = &x[from..from + n];
    let (b0, b1) = ((lo / rate * n as f32) as usize, (hi / rate * n as f32) as usize);
    let mut e = 0f64;
    for b in b0..b1.min(n / 2) {
        let (mut re, mut im) = (0f64, 0f64);
        for (i, &s) in seg.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos();
            let ph = 2.0 * std::f64::consts::PI * (b * i) as f64 / n as f64;
            re += s as f64 * w * ph.cos();
            im += s as f64 * w * ph.sin();
        }
        e += re * re + im * im;
    }
    10.0 * (e + 1e-12).log10()
}

struct Case {
    name: &'static str,
    rate: u32,
    channels: usize,
    bitrate: &'static str,
}

/// HE-AAC streams of 1 to 6 channels at 22.05 to 48 kHz decode as ffmpeg decodes them: the same
/// timing, the low band to 55 dB and the rebuilt high band's level to 0.1 dB.
#[test]
fn matches_ffmpeg_he_aac_decoder() {
    let Some(ff) = ffmpeg_with_aac_at() else { return };
    let cases = [
        Case { name: "he_stereo_44k_48k", rate: 44_100, channels: 2, bitrate: "48k" },
        Case { name: "he_mono_44k_32k", rate: 44_100, channels: 1, bitrate: "32k" },
        Case { name: "he_stereo_48k_64k", rate: 48_000, channels: 2, bitrate: "64k" },
        Case { name: "he_stereo_32k_32k", rate: 32_000, channels: 2, bitrate: "32k" },
        Case { name: "he_stereo_44k_24k", rate: 44_100, channels: 2, bitrate: "24k" },
        Case { name: "he_stereo_22k_24k", rate: 22_050, channels: 2, bitrate: "24k" },
        Case { name: "he_51_48k_160k", rate: 48_000, channels: 6, bitrate: "160k" },
    ];
    for case in &cases {
        let path = fixtures().join(format!("{}.aac", case.name));
        ffmpeg_encode(&ff, &input(case.rate, case.channels, 3.0), case.rate, &["-c:a", "aac_at", "-profile:a", "4", "-b:a", case.bitrate], &path);
        let data = std::fs::read(&path).unwrap();
        let refr = ffmpeg_decode_as(&ff, &path, case.channels);
        let (ours, rate) = decode_sbr(&data);
        // ffmpeg puts surround in WAV order, the decoder in AAC element order
        let ours: Vec<Vec<f32>> = match (case.channels == 6).then(|| craft_audio::aac::wav_order(6)).flatten() {
            Some(order) => order.iter().map(|&c| ours[c].clone()).collect(),
            None => ours,
        };
        assert_eq!(rate, case.rate, "{}", case.name);
        assert_eq!(ours.len(), case.channels);
        // the same delay as ffmpeg's decoder: 962 samples
        assert_eq!(best_lag(&refr[0], &ours[0], 2100), 0, "{}", case.name);
        let core_nyquist = case.rate as f32 / 4.0;
        for c in 0..case.channels {
            let n = refr[c].len().min(ours[c].len()) - 4096;
            let a: Vec<f32> = refr[c][2048..n].to_vec();
            let b: Vec<f32> = ours[c][2048..n].to_vec();
            let full = snr(&a, &b);
            let (la, lb) = (lowpass(&a, core_nyquist * 0.8, case.rate as f32), lowpass(&b, core_nyquist * 0.8, case.rate as f32));
            let low = snr(&la, &lb);
            let mut diffs = Vec::new();
            for from in (4096..a.len() - 4096).step_by(4096) {
                for (lo, hi) in
                    [(core_nyquist * 1.05, core_nyquist * 1.3), (core_nyquist * 1.3, core_nyquist * 1.7), (core_nyquist * 1.7, core_nyquist * 2.0)]
                {
                    let (ea, eb) = (band_db(&a, from, case.rate as f32, lo, hi), band_db(&b, from, case.rate as f32, lo, hi));
                    if ea > 0.0 {
                        diffs.push(eb - ea);
                    }
                }
            }
            // the LFE (WAV channel 3 of 5.1) has no SBR, so no high band; every other channel must
            // have one to compare
            let lfe = case.channels == 6 && c == 3;
            assert!(lfe || !diffs.is_empty(), "{} ch{c}: no high band to compare", case.name);
            let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
            let mad = diffs.iter().map(|d| d.abs()).sum::<f64>() / diffs.len() as f64;
            println!(
                "{} ch{c}: full SNR {full:.1} dB, low band SNR {low:.1} dB, high band level {mean:+.2} dB (mean |Δ| {mad:.2} dB over {})",
                case.name,
                diffs.len()
            );
            // The low band differs by the core decoders' rounding; the high band's noise comes
            // from the same table, so whole frames match too.
            assert!(full > 45.0 && low > 55.0, "{} ch{c}: SNR {full:.1} / {low:.1} dB", case.name);
            assert!(lfe || mad < 0.1, "{} ch{c}: high band off by {mad:.2} dB", case.name);
        }
    }
}
