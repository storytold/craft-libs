//! The Vorbis decoder against ffmpeg's as the reference (oracle; skipped when ffmpeg is absent, a
//! failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`). Streams come from two encoders, this crate's and
//! ffmpeg's own, so the setups differ: different floors, residues, block sizes, channel counts and
//! coupling. Both decoders must return the same samples.

use craft_audio::AudioBuffer;
use craft_audio::vorbis::decoder::{Decoder, decode_ogg};
use craft_audio::{OggVorbisOptions, encode_ogg_vorbis};
use std::path::Path;
use std::process::Command;

fn xorshift(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

/// A signal with tones, noise bursts, clicks and silence, different in every channel.
fn signal(rate: u32, channels: usize, frames: usize) -> Vec<Vec<f32>> {
    let mut seed = 0x1234_5678u32;
    (0..channels)
        .map(|c| {
            (0..frames)
                .map(|i| {
                    let t = i as f64 / f64::from(rate);
                    let tone =
                        (t * (220.0 + 110.0 * c as f64) * std::f64::consts::TAU).sin() * 0.3 + (t * 3100.0 * std::f64::consts::TAU).sin() * 0.05;
                    let noise = (f64::from(xorshift(&mut seed) % 2000) - 1000.0) / 6000.0;
                    let s = match (i / (rate as usize / 4)) % 6 {
                        0 | 1 => tone,
                        2 => noise,
                        3 => 0.0,
                        4 => tone + if i % 3000 < 8 { 0.6 } else { 0.0 },
                        _ => tone * (t * 4.0).sin(),
                    };
                    s as f32
                })
                .collect()
        })
        .collect()
}

fn ffmpeg_decode(ffmpeg: &Path, ogg: &Path, channels: usize) -> Vec<Vec<f32>> {
    let out = Command::new(ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(ogg)
        .args(["-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .output()
        .expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg failed on {}: {}", ogg.display(), String::from_utf8_lossy(&out.stderr));
    let all: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    (0..channels).map(|c| all.iter().skip(c).step_by(channels).copied().collect()).collect()
}

/// Each of our channels matches a different one of theirs (ffmpeg reorders the channels of surround
/// streams), to within float rounding.
fn assert_same(name: &str, ours: &[Vec<f32>], theirs: &[Vec<f32>]) {
    assert_prefix(name, ours, theirs, true);
}

/// As [`assert_same`], but with `exact` unset ffmpeg may return more samples than we do (it does
/// not always trim a stream to its last granule position); the common part must agree.
fn assert_prefix(name: &str, ours: &[Vec<f32>], theirs: &[Vec<f32>], exact: bool) {
    assert_eq!(ours.len(), theirs.len(), "{name}: channel count");
    assert!(ours.iter().chain(theirs).flatten().all(|v| v.is_finite()), "{name}: non-finite oracle input or output");
    let mut used = vec![false; theirs.len()];
    for (c, o) in ours.iter().enumerate() {
        let mut best: Option<(usize, f32)> = None;
        for (k, t) in theirs.iter().enumerate() {
            if used[k] {
                continue;
            }
            if exact {
                assert_eq!(o.len(), t.len(), "{name}: channel {c} length {} vs ffmpeg {}", o.len(), t.len());
            } else {
                assert!(o.len() <= t.len(), "{name}: channel {c} length {} vs ffmpeg {}", o.len(), t.len());
            }
            let worst = o.iter().zip(t).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            if best.is_none_or(|(_, w)| worst < w) {
                best = Some((k, worst));
            }
        }
        let (k, worst) = best.expect("a free channel");
        used[k] = true;
        assert!(worst < 2e-4, "{name}: channel {c} differs from ffmpeg's channel {k} by up to {worst}");
    }
}

#[test]
fn ffmpeg_decodes_our_encoders_streams_to_the_same_samples() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("vorbis_decode");
    for (rate, channels, quality) in
        [(44_100u32, 2usize, 6.0f32), (44_100, 1, 3.0), (48_000, 2, -1.0), (22_050, 1, 10.0), (44_100, 6, 5.0), (32_000, 8, 4.0)]
    {
        let buf = AudioBuffer { sample_rate: rate, channels: signal(rate, channels, rate as usize * 2 + 777) };
        let ogg = encode_ogg_vorbis(&buf, &OggVorbisOptions::default().with_quality(quality)).expect("encode");
        let file = dir.join(format!("ours_{rate}_{channels}_{quality}.ogg"));
        std::fs::write(&file, &ogg).expect("write fixture");
        let decoded = decode_ogg(&ogg).expect("decode");
        assert_eq!(decoded.damaged_packets, 0);
        assert_eq!(decoded.sample_rate, rate);
        assert_same(&format!("our stream {rate} Hz {channels} ch q{quality}"), &decoded.channels, &ffmpeg_decode(&ffmpeg, &file, channels));
    }
}

/// ffmpeg's own Vorbis encoder (stereo only) writes setups ours does not: coupled channels, residue
/// type 2, floor classes with subclasses and master books, two block sizes. The reference is
/// Symphonia, an independent decoder. ffmpeg's decoder agrees on the first channel but not on the
/// second of these streams, where it differs from both Symphonia and this decoder; the stream does
/// not show which reading it means (the channel comes back poorly, about 3 dB SNR, in all three),
/// so ffmpeg is compared on the first channel only. The setups ffmpeg's decoder does read the same
/// way as we do are covered, on both channels, in `vorbis_decode_synthetic.rs`.
#[test]
fn we_decode_ffmpegs_encoder_streams_like_symphonia() {
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("vorbis_decode");
    for (rate, bitrate) in [(44_100u32, "128k"), (48_000, "96k"), (22_050, "48k"), (32_000, "64k")] {
        let channels = 2usize;
        let pcm: Vec<f32> = {
            let s = signal(rate, channels, rate as usize * 2 + 301);
            (0..s[0].len()).flat_map(|i| s.iter().map(move |c| c[i])).collect()
        };
        let raw = dir.join(format!("src_{rate}_{channels}.f32"));
        std::fs::write(&raw, pcm.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).expect("write source");
        let file = dir.join(format!("ffmpeg_{rate}_{channels}_{bitrate}.ogg"));
        let out = Command::new(&ffmpeg)
            .args(["-v", "error", "-nostdin", "-y", "-f", "f32le", "-ar"])
            .arg(rate.to_string())
            .arg("-ac")
            .arg(channels.to_string())
            .arg("-i")
            .arg(&raw)
            .args(["-c:a", "vorbis", "-strict", "-2", "-b:a", bitrate])
            .arg(&file)
            .output()
            .expect("run ffmpeg");
        assert!(out.status.success(), "ffmpeg could not encode {}: {}", file.display(), String::from_utf8_lossy(&out.stderr));
        let ogg = std::fs::read(&file).expect("read");
        let ours = decode_ogg(&ogg).expect("decode");
        assert_eq!(ours.damaged_packets, 0, "{}", file.display());
        let (_, symphonia) = craft_audio::decode(&ogg, Some("ogg")).expect("symphonia");
        let name = format!("ffmpeg-encoded {rate} Hz {bitrate}");
        assert_prefix(&name, &ours.channels, &symphonia.channels, false);
        // The first channel also agrees with ffmpeg's decoder.
        let theirs = ffmpeg_decode(&ffmpeg, &file, channels);
        assert_prefix(&format!("{name} (channel 0 against ffmpeg)"), &ours.channels[..1], &theirs[..1], true);
    }
}

#[test]
fn packets_decode_one_at_a_time_to_the_file_result() {
    let buf = AudioBuffer { sample_rate: 44_100, channels: signal(44_100, 2, 30_000) };
    let ogg = encode_ogg_vorbis(&buf, &OggVorbisOptions::default().with_quality(5.0)).expect("encode");
    let file = craft_media::ogg::open(ogg.as_slice()).expect("open");
    let s = &file.streams[file.stream_of(craft_media::ogg::Codec::Vorbis).expect("vorbis")];
    let mut dec = Decoder::new(&s.headers[0], &s.headers[1], &s.headers[2]).expect("decoder");
    assert_eq!((dec.identification().channels, dec.identification().sample_rate), (2, 44_100));
    let mut got: Vec<Vec<f32>> = vec![Vec::new(); 2];
    for i in 0..s.packets.len() {
        let p = file.read_packet(ogg.as_slice(), 0, i).expect("packet");
        for (a, b) in got.iter_mut().zip(dec.decode_packet(&p).expect("packet decodes")) {
            a.extend(b);
        }
    }
    let whole = decode_ogg(&ogg).expect("file");
    for (a, b) in got.iter().zip(&whole.channels) {
        assert!(a.len() >= b.len() && a[..b.len()] == b[..]);
    }
    // After a reset the next packet only primes the overlap again.
    dec.reset();
    let p = file.read_packet(ogg.as_slice(), 0, 5).expect("packet");
    assert!(dec.decode_packet(&p).expect("decodes").iter().all(Vec::is_empty));
}

#[test]
fn the_round_trip_through_our_encoder_is_faithful() {
    let src = signal(44_100, 2, 44_100);
    let buf = AudioBuffer { sample_rate: 44_100, channels: src.clone() };
    let ogg = encode_ogg_vorbis(&buf, &OggVorbisOptions::default().with_quality(8.0)).expect("encode");
    let out = decode_ogg(&ogg).expect("decode");
    assert_eq!(out.channels[0].len(), src[0].len());
    let (mut sig, mut err) = (0f64, 0f64);
    for (a, b) in src[0].iter().zip(&out.channels[0]) {
        sig += f64::from(*a) * f64::from(*a);
        err += f64::from(a - b) * f64::from(a - b);
    }
    assert!(10.0 * (sig / err).log10() > 15.0, "SNR {:.1} dB", 10.0 * (sig / err).log10());
}

/// A stream from libvorbis itself (the repository's `tone.ogg` fixture, made with ffmpeg's libvorbis
/// wrapper): mono, 8 kHz, the lowest quality.
#[test]
fn a_libvorbis_stream_decodes_like_ffmpeg() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.ogg");
    let ogg = std::fs::read(&path).expect("fixture");
    let ours = decode_ogg(&ogg).expect("decode");
    assert_eq!((ours.sample_rate, ours.channels.len(), ours.damaged_packets), (8000, 1, 0));
    assert!(ours.comments.vendor.contains("Lavf") || ours.comments.vendor.contains("libVorbis"), "vendor {:?}", ours.comments.vendor);
    assert!(ours.channels[0].iter().any(|s| s.abs() > 0.1), "the fixture is a tone");
    let Some(ffmpeg) = craft_media_testkit::ffmpeg() else { return };
    assert_prefix("libvorbis tone.ogg", &ours.channels, &ffmpeg_decode(&ffmpeg, &path, 1), false);
}

#[test]
#[should_panic(expected = "non-finite oracle input or output")]
fn oracle_comparison_rejects_nan_output() {
    assert_same("NaN output", &[vec![f32::NAN]], &[vec![0.0]]);
}

#[test]
#[should_panic(expected = "non-finite oracle input or output")]
fn oracle_comparison_rejects_nan_reference() {
    assert_same("NaN reference", &[vec![0.0]], &[vec![f32::NAN]]);
}
