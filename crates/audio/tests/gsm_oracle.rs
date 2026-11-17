//! GSM 06.10 against ffmpeg's decoder as the reference (oracle; skipped when ffmpeg is absent, a
//! failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`): ffmpeg decodes what our encoder writes, in both
//! packings, to exactly the samples our decoder returns, and so does arbitrary frame data.

use craft_audio::gsm::{Packing, decode, encode};
use std::path::Path;
use std::process::Command;

fn xorshift(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

fn signal(frames: usize) -> Vec<i16> {
    let mut seed = 0x9E37_79B9u32;
    (0..frames * 160)
        .map(|i| {
            let t = i as f64 / 8000.0;
            let f0 = 110.0 + 40.0 * (t * 2.0).sin();
            let voiced = (t * f0 * std::f64::consts::TAU).sin() * 7000.0 + (t * 4.0 * f0 * std::f64::consts::TAU).sin() * 2000.0;
            let noise = f64::from(xorshift(&mut seed) % 3000) - 1500.0;
            // Voiced stretches, noise bursts, silence and full-scale clipping.
            match (i / 1600) % 5 {
                0 | 1 => voiced as i16,
                2 => noise as i16,
                3 => 0,
                _ => (voiced * 6.0).clamp(-32768.0, 32767.0) as i16,
            }
        })
        .collect()
}

fn wav49_file(blocks: &[u8], samples: usize) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&0x31u16.to_le_bytes()); // GSM 6.10
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&8000u32.to_le_bytes());
    fmt.extend_from_slice(&1625u32.to_le_bytes());
    fmt.extend_from_slice(&65u16.to_le_bytes());
    fmt.extend_from_slice(&0u16.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&320u16.to_le_bytes());
    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(b"fmt ");
    body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    body.extend_from_slice(&fmt);
    body.extend_from_slice(b"fact");
    body.extend_from_slice(&4u32.to_le_bytes());
    body.extend_from_slice(&(samples as u32).to_le_bytes());
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    body.extend_from_slice(blocks);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

fn ffmpeg_decode(ffmpeg: &Path, file: &Path, forced_format: Option<&str>) -> Vec<i16> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", "error", "-y"]);
    if let Some(f) = forced_format {
        cmd.args(["-f", f]);
    }
    let out = cmd.arg("-i").arg(file).args(["-f", "s16le", "-acodec", "pcm_s16le", "-"]).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg failed on {}: {}", file.display(), String::from_utf8_lossy(&out.stderr));
    out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

/// Frames whose long-term and RPE fields are arbitrary, over the log-area ratios of real speech.
/// Random log-area ratios give filters that overflow at every sample, where the specification's
/// saturating arithmetic and other decoders' differ; here every field of every sub-frame still
/// takes all of its values, with the amplitude and gain kept where the signal stays in range.
fn random_frames(n: usize, seed: &mut u32) -> Vec<u8> {
    let speech = encode(&signal(n), Packing::Raw);
    let mut out = Vec::new();
    for b in speech.as_chunks::<33>().0 {
        let mut frame = craft_audio::gsm::Frame::from_raw(b).expect("frame");
        for s in &mut frame.sub {
            s.lag = 40 + (xorshift(seed) % 81) as u8;
            s.gain = (xorshift(seed) % 3) as u8;
            s.grid = (xorshift(seed) & 3) as u8;
            s.xmax = (xorshift(seed) % 26) as u8;
            for p in &mut s.pulses {
                *p = (xorshift(seed) & 7) as u8;
            }
        }
        out.extend_from_slice(&frame.to_raw());
    }
    out
}

#[test]
fn ffmpeg_decodes_our_raw_streams_to_the_same_samples() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("gsm");
    for frames in [1usize, 2, 7, 50, 120] {
        let bytes = encode(&signal(frames), Packing::Raw);
        let file = dir.join(format!("ours_{frames}.gsm"));
        std::fs::write(&file, &bytes).expect("write fixture");
        let theirs = ffmpeg_decode(&ffmpeg, &file, Some("gsm"));
        let ours = decode(&bytes, Packing::Raw).expect("decode");
        assert_eq!(theirs.len(), ours.len(), "{frames} frames: sample count");
        assert!(theirs == ours, "{frames} frames: samples differ");
    }
}

#[test]
fn ffmpeg_decodes_our_wav49_streams_to_the_same_samples() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("gsm");
    for frames in [2usize, 4, 20, 100] {
        let pcm = signal(frames);
        let bytes = encode(&pcm, Packing::Wav49);
        let file = dir.join(format!("ours_{frames}.wav"));
        std::fs::write(&file, wav49_file(&bytes, pcm.len())).expect("write fixture");
        let theirs = ffmpeg_decode(&ffmpeg, &file, None);
        let ours = decode(&bytes, Packing::Wav49).expect("decode");
        assert_eq!(theirs.len(), ours.len(), "{frames} frames: sample count");
        assert!(theirs == ours, "{frames} frames: samples differ");
    }
}

#[test]
fn ffmpeg_and_we_decode_arbitrary_frames_identically() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("gsm");
    let mut seed = 0xC0FF_EE11u32;
    let bytes = random_frames(300, &mut seed);
    let file = dir.join("random.gsm");
    std::fs::write(&file, &bytes).expect("write fixture");
    let theirs = ffmpeg_decode(&ffmpeg, &file, Some("gsm"));
    let ours = decode(&bytes, Packing::Raw).expect("decode");
    assert_eq!(theirs.len(), ours.len());
    assert!(theirs == ours, "arbitrary frames decode differently");
}
