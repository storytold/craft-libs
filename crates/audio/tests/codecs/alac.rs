//! ALAC against ffmpeg (oracle; skipped when ffmpeg is absent): ffmpeg's ALAC files decode to the
//! exact input samples through `craft_audio::decode`, and ffmpeg decodes our encoder's packets,
//! muxed into M4A, to the exact input samples. Both in WAV channel order.

use craft_audio::alac::{Encoder, EncoderConfig, alac_order};
use craft_audio::craft_media::isobmff::{Brand, CodecConfig, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use craft_audio::{CodecId, decode};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

fn dir() -> PathBuf {
    crate::testkit::fixtures_dir("alac")
}

/// The ffmpeg channel layout ALAC uses for `channels`.
fn layout(channels: usize) -> &'static str {
    ["mono", "stereo", "3.0", "4.0", "5.0", "5.1", "6.1(back)", "7.1(wide)"][channels - 1]
}

/// `frames` frames of `channels` channels at `bits` bits, in WAV order: tones, noise bursts,
/// silence and full-scale peaks.
fn input(channels: usize, bits: u32, frames: usize) -> Vec<Vec<i32>> {
    let max = ((1i64 << (bits - 1)) - 1) as f64;
    let mut seed = 0x2545_F491u32;
    (0..channels)
        .map(|c| {
            (0..frames)
                .map(|i| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let noise = seed as f64 / f64::from(u32::MAX) * 2.0 - 1.0;
                    let tone = (i as f64 * (200.0 + 50.0 * c as f64) / 48_000.0 * std::f64::consts::TAU).sin();
                    let x = match (i / 2500) % 4 {
                        0 => tone * 0.7 + noise * 0.001,
                        1 => noise * 0.5,
                        2 => 0.0,
                        _ => tone.signum(),
                    };
                    (x * max).round().clamp(-max - 1.0, max) as i32
                })
                .collect()
        })
        .collect()
}

/// Interleaved s32le with each sample at the top of the word.
fn s32le(planar: &[Vec<i32>], bits: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..planar[0].len() {
        for c in planar {
            out.extend_from_slice(&(c[i] << (32 - bits)).to_le_bytes());
        }
    }
    out
}

/// ffmpeg's decode of `file` as planar samples at `bits` bits.
fn ffmpeg_decode(ff: &Path, file: &Path, channels: usize, bits: u32) -> Vec<Vec<i32>> {
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-f", "s32le", "-"]).output().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
    let mut planar = vec![Vec::new(); channels];
    for (i, w) in out.stdout.as_chunks::<4>().0.iter().enumerate() {
        planar[i % channels].push(i32::from_le_bytes([w[0], w[1], w[2], w[3]]) >> (32 - bits));
    }
    planar
}

/// Our encoder's packets for `wav` (WAV order) muxed into M4A.
fn our_m4a(wav: &[Vec<i32>], bits: u32, frame_length: u32) -> Vec<u8> {
    let channels = wav.len();
    let alac: Vec<&[i32]> = match alac_order(channels as u8) {
        Some(order) => order.iter().map(|&w| wav[w].as_slice()).collect(),
        None => wav.iter().map(Vec::as_slice).collect(),
    };
    let mut enc = Encoder::new(EncoderConfig::new(48_000, channels as u8, bits as u8).with_frame_length(frame_length)).unwrap();
    let frames = wav[0].len();
    let packets: Vec<(Vec<u8>, u32)> = (0..frames)
        .step_by(frame_length as usize)
        .map(|s| {
            let e = (s + frame_length as usize).min(frames);
            (enc.encode(&alac.iter().map(|c| &c[s..e]).collect::<Vec<_>>()).unwrap(), (e - s) as u32)
        })
        .collect();
    let entry = SampleEntry::audio(FourCc(*b"alac"), CodecConfig::Alac { cookie: enc.cookie() }, channels as u32, 48_000.0, bits as u16);
    let mut w = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let t = w.add_track(TrackConfig::new(entry, 48_000)).unwrap();
    for (data, duration) in &packets {
        w.write_sample(t, WriteSample { data, duration: *duration, composition_offset: 0, is_sync: true }).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// `craft_audio::decode` of an ALAC file, back to integers (exact up to 24 bits, which `f32`
/// holds).
fn our_decode(bytes: &[u8], bits: u32) -> Vec<Vec<i32>> {
    let (info, buf) = decode(bytes, Some("m4a")).unwrap();
    assert_eq!((info.codec, info.damaged), (CodecId::Alac, 0));
    let scale = 2f64.powi(bits as i32 - 1);
    buf.channels.iter().map(|c| c.iter().map(|&s| (f64::from(s) * scale) as i32).collect()).collect()
}

#[test]
fn ffmpeg_alac_decodes_exactly() {
    let Some(ff) = crate::testkit::ffmpeg_or_skip("alac oracle") else { return };
    for (channels, bits) in [(1, 16), (2, 16), (2, 24), (3, 16), (4, 16), (5, 24), (6, 24), (7, 16), (8, 16)] {
        let wav = input(channels, bits, 12_345);
        let name = format!("ffmpeg_{channels}ch_{bits}");
        let raw = dir().join(format!("{name}.s32"));
        let m4a = dir().join(format!("{name}.m4a"));
        std::fs::write(&raw, s32le(&wav, bits)).unwrap();
        let fmt = if bits == 16 { "s16p" } else { "s32p" };
        let st = Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "s32le", "-ar", "48000", "-ch_layout", layout(channels), "-i"])
            .arg(&raw)
            .args(["-c:a", "alac", "-sample_fmt", fmt])
            .arg(&m4a)
            .status()
            .unwrap();
        assert!(st.success(), "{name}: ffmpeg encode");
        let ours = our_decode(&std::fs::read(&m4a).unwrap(), bits);
        assert!(ours == wav, "{name}: decoded samples differ from the input");
    }
}

/// 32-bit streams are left out: ffmpeg does not decode escaped (uncompressed) 32-bit stereo
/// frames ("not yet implemented"), which the noise in the input produces; Apple's decoder does.
#[test]
fn ffmpeg_decodes_our_alac_exactly() {
    let Some(ff) = crate::testkit::ffmpeg_or_skip("alac oracle") else { return };
    for (channels, bits, frame) in [
        (1, 16, 4096),
        (2, 16, 4096),
        (2, 20, 4096),
        (2, 24, 1000),
        (3, 16, 4096),
        (4, 24, 4096),
        (5, 16, 4096),
        (6, 24, 4096),
        (7, 16, 4096),
        (8, 24, 4096),
    ] {
        let wav = input(channels, bits, 10_007);
        let m4a = our_m4a(&wav, bits, frame);
        let path = dir().join(format!("ours_{channels}ch_{bits}.m4a"));
        std::fs::write(&path, &m4a).unwrap();
        let theirs = ffmpeg_decode(&ff, &path, channels, bits);
        assert!(theirs == wav, "{channels} ch, {bits} bits: ffmpeg's decode differs from the input");
        // and our own reader on the same file
        if bits <= 24 {
            assert!(our_decode(&m4a, bits) == wav, "{channels} ch, {bits} bits: our decode differs");
        }
    }
}
