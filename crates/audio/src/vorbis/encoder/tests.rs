//! Unit tests, and ffmpeg (as an external oracle only) decoding the encoder's streams.

#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::process::Command;

use super::book::{BitWriter, assign_codewords, float32_pack};
use super::{BLOCK, Encoder, EncoderConfig, HOP, partition_size};

#[test]
fn codewords_follow_the_specification_example() {
    // Vorbis I §3.2.1: lengths 2 4 4 4 4 2 3 3
    let words = assign_codewords(&[2, 4, 4, 4, 4, 2, 3, 3]).unwrap();
    assert_eq!(words, [0b00, 0b0100, 0b0101, 0b0110, 0b0111, 0b10, 0b110, 0b111]);
    assert!(assign_codewords(&[1, 1, 1]).is_err(), "over-subscribed");
}

#[test]
fn every_book_is_a_complete_prefix_code() {
    let e = Encoder::new(EncoderConfig::new(48_000, 2, 5.0)).unwrap();
    for (i, b) in e.books.iter().enumerate() {
        let kraft: f64 = b.lengths.iter().map(|&l| 0.5f64.powi(l as i32)).sum();
        assert!((kraft - 1.0).abs() < 1e-9, "book {i}: Kraft sum {kraft}");
        assert!(b.lengths.iter().all(|&l| (1..=20).contains(&l)));
        for (a, (&wa, &la)) in b.words.iter().zip(&b.lengths).enumerate() {
            for (c, (&wc, &lc)) in b.words.iter().zip(&b.lengths).enumerate() {
                if a != c && la <= lc {
                    assert_ne!(wc >> (lc - la), wa, "book {i}: codeword {a} is a prefix of {c}");
                }
            }
        }
    }
}

#[test]
fn float32_pack_is_exact_for_lattice_values() {
    let unpack = |x: u32| {
        let mant = f64::from(x & 0x1F_FFFF);
        let exp = ((x >> 21) & 0x3FF) as i32 - 788;
        let v = mant * 2f64.powi(exp);
        if x & 0x8000_0000 != 0 { -v } else { v }
    };
    for v in [1.0, -1.0, -2.0, -4.0, -7.0, -105.0, 15.0, -1575.0, 225.0, 0.5, 3.0] {
        assert_eq!(unpack(float32_pack(v)), v);
    }
    assert_eq!(float32_pack(0.0), 0);
}

#[test]
fn bits_are_packed_lsb_first() {
    let mut w = BitWriter::new();
    w.write(0b1, 1);
    w.write(0b10, 2);
    w.write(0x1F, 5);
    w.write(0xABC, 12);
    assert_eq!(w.finish(), [0b1111_1101, 0xBC, 0x0A]);
}

#[test]
fn hostile_input_is_an_error_not_a_panic() {
    assert!(Encoder::new(EncoderConfig::new(48_000, 0, 3.0)).is_err());
    assert!(Encoder::new(EncoderConfig::new(48_000, 9, 3.0)).is_err());
    assert!(Encoder::new(EncoderConfig::new(0, 2, 3.0)).is_err());
    assert_eq!(Encoder::new(EncoderConfig::new(48_000, 1, f32::NAN)).unwrap().cfg.quality, 6.0, "NaN quality uses the default, as the export does");
    let mut e = Encoder::new(EncoderConfig::new(44_100, 2, 3.0)).unwrap();
    assert!(e.encode(&[&[0.0; 4]]).is_err());
    assert!(e.encode(&[&[0.0; 4], &[0.0; 3]]).is_err());
    assert!(e.encode(&[&[f32::NAN; 4], &[0.0; 4]]).is_err());
    let loud = vec![1e30f32; 5000];
    assert!(!e.encode(&[&loud, &loud]).unwrap().is_empty());
    assert!(!e.finish().unwrap().is_empty());
    assert!(e.finish().unwrap().is_empty());
    assert!(e.encode(&[&[0.0; 4], &[0.0; 4]]).is_err());
}

#[test]
fn granules_count_the_samples_and_trim_the_end() {
    for len in [0usize, 1, 1023, 1024, 1025, 48_000] {
        let mut e = Encoder::new(EncoderConfig::new(48_000, 1, 3.0)).unwrap();
        let x = vec![0.1f32; len];
        let mut packets = e.encode(&[&x]).unwrap();
        packets.extend(e.finish().unwrap());
        assert_eq!(packets.len(), len.div_ceil(HOP) + 1, "len {len}");
        // packet k ends output at k·1024 samples; the last one at the input's length
        let want: Vec<i64> = (0..packets.len()).map(|k| ((k * HOP) as i64).min(len as i64)).collect();
        let got: Vec<i64> = packets.iter().map(|p| p.1).collect();
        assert_eq!(got, want, "len {len}");
    }
}

// --- ffmpeg oracle ---

/// An Ogg Vorbis file of the three headers and the audio packets, through craft-media's writer.
fn ogg(packets: &[(Vec<u8>, i64)]) -> Vec<u8> {
    let mut w = craft_media::ogg::PageWriter::new(7);
    let mut out = Vec::new();
    for (i, (p, g)) in packets.iter().enumerate() {
        out.extend(if i < 3 { w.header(p) } else { w.packet(p, *g) });
    }
    out.extend(w.finish(None));
    out
}

fn comment() -> Vec<u8> {
    craft_media::ogg::vorbis_comment("test", &[])
}

/// Encodes `input` (planar) and decodes it with ffmpeg's `decoder`; returns the planar output.
/// ffmpeg's decoders to check: its own `vorbis` always, and `libvorbis` when ffmpeg has it.
fn decoders(ff: &Path) -> Vec<&'static str> {
    let list =
        Command::new(ff).args(["-hide_banner", "-decoders"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    if list.lines().any(|l| l.split_whitespace().nth(1) == Some("libvorbis")) {
        vec!["vorbis", "libvorbis"]
    } else {
        eprintln!("SKIPPED (libvorbis decoder): this ffmpeg has no libvorbis; checking its own decoder only");
        vec!["vorbis"]
    }
}

fn round_trip(input: &[Vec<f32>], rate: u32, quality: f32, decoder: &str, name: &str) -> Option<(Vec<Vec<f32>>, usize)> {
    let ff = craft_media_testkit::ffmpeg_or_skip("vorbis oracle")?;
    let mut e = Encoder::new(EncoderConfig::new(rate, input.len(), quality)).unwrap();
    let mut packets = vec![(e.identification(), 0), (comment(), 0), (e.setup(), 0)];
    let n = input[0].len();
    let mut at = 0;
    while at < n {
        let end = (at + 4410).min(n);
        let s: Vec<&[f32]> = input.iter().map(|c| &c[at..end]).collect();
        packets.extend(e.encode(&s).unwrap());
        at = end;
    }
    packets.extend(e.finish().unwrap());
    let bytes: usize = packets[3..].iter().map(|p| p.0.len()).sum();
    let dir = craft_media_testkit::fixtures_dir("audio/vorbis-encoder");
    let path = dir.join(format!("{name}.ogg"));
    std::fs::write(&path, ogg(&packets)).unwrap();
    let out = Command::new(ff)
        .args(["-v", "error", "-xerror", "-c:a", decoder, "-i"])
        .arg(&path)
        .args(["-f", "f32le", "-c:a", "pcm_f32le", "-"])
        .output()
        .unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{name} {decoder}: {}", String::from_utf8_lossy(&out.stderr));
    let pcm: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect();
    let ch = input.len();
    Some(((0..ch).map(|c| pcm.iter().skip(c).step_by(ch).copied().collect()).collect(), bytes))
}

fn snr_db(a: &[f32], b: &[f32]) -> f64 {
    let (mut s, mut e) = (0f64, 0f64);
    for (&x, &y) in a.iter().zip(b) {
        s += f64::from(x) * f64::from(x);
        e += (f64::from(x) - f64::from(y)).powi(2);
    }
    10.0 * (s.max(1e-20) / e.max(1e-20)).log10()
}

fn music(n: usize, seed: u32) -> Vec<f32> {
    let mut r = seed;
    (0..n)
        .map(|i| {
            let t = i as f32 / 48_000.0;
            r = r.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (r >> 8) as f32 / 8_388_608.0 - 1.0;
            let chord: f32 = [220.0f32, 277.18, 329.63, 440.0]
                .iter()
                .enumerate()
                .map(|(k, f)| (0.2 / (k + 1) as f32) * (2.0 * std::f32::consts::PI * f * t).sin())
                .sum();
            chord + 0.01 * noise
        })
        .collect()
}

/// Harmonic chords changing every half second, with decaying noise bursts (music-like).
fn chords(n: usize, seed: u32) -> Vec<f32> {
    let mut r = seed;
    let notes = [220.0f32, 277.2, 329.6, 440.0, 349.2, 523.3, 392.0, 659.3];
    (0..n)
        .map(|i| {
            let t = i as f32 / 48_000.0;
            let chord = (t * 2.0) as usize;
            let mut s = 0.0;
            for k in 0..3 {
                let f = notes[(chord * 3 + k * 2) % notes.len()];
                for h in 1..6 {
                    s += (2.0 * std::f32::consts::PI * f * h as f32 * t).sin() * 0.08 / h as f32;
                }
            }
            r = r.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let env = (-(t * 2.0).fract() * 3.0).exp();
            s * (0.5 + 0.5 * env) + 0.02 * ((r >> 8) as f32 / 8_388_608.0 - 1.0) * env
        })
        .collect()
}

/// ffmpeg's own Vorbis decoder and libvorbis read every stream exactly as long as the input,
/// with SNR floors about 6 dB (chords 4 dB) under what the encoder measured when written, and bit
/// rates near libvorbis's for the same quality.
/// A signal, and per quality the SNR floor and the bit-rate range (kbit/s).
type Case = (&'static str, Vec<Vec<f32>>, [(f32, f64, std::ops::Range<usize>); 5]);

#[test]
fn ffmpeg_decodes_every_quality() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("vorbis oracle") else { return };
    let n = 48_000 * 3;
    let cases: [Case; 2] = [
        (
            "music",
            vec![music(n, 1), music(n, 2).iter().map(|v| v * 0.7).collect()],
            [(-1.0, 3.0, 15..90), (0.0, 6.5, 20..110), (3.0, 12.0, 60..180), (6.0, 18.5, 120..280), (10.0, 25.5, 180..400)],
        ),
        (
            "chords",
            vec![chords(n, 1), chords(n, 2).iter().map(|v| v * 0.8).collect()],
            [(-1.0, 4.5, 20..90), (0.0, 6.0, 25..110), (3.0, 10.0, 70..200), (6.0, 14.0, 130..300), (10.0, 19.0, 180..420)],
        ),
    ];
    for (name, input, levels) in &cases {
        for (q, floor, kbps) in levels {
            for decoder in decoders(&ff) {
                let Some((out, bytes)) = round_trip(input, 48_000, *q, decoder, &format!("{name}{q}")) else { return };
                assert!(out.iter().all(|c| c.len() == n), "{name} q{q} {decoder}: length {}", out[0].len());
                let snr = snr_db(&input[0][4096..], &out[0][4096..]);
                assert!(snr >= *floor, "{name} q{q} {decoder}: {snr:.2} dB < {floor}");
                let rate = bytes * 8 / 3 / 1000;
                assert!(kbps.contains(&rate), "{name} q{q}: {rate} kbit/s outside {kbps:?}");
            }
        }
    }
}

/// Mono, 44.1 kHz, 3.0, 5.1 and 6.1 streams decode too, with libvorbis as well (which reads each
/// residue partition from channel 0); a silent channel costs one bit per block.
#[test]
fn channel_counts_and_rates() {
    let Some(ff) = craft_media_testkit::ffmpeg_or_skip("vorbis oracle") else { return };
    let n = 44_100;
    for (rate, ch) in [(44_100u32, 1usize), (22_050, 2), (48_000, 3), (48_000, 6), (48_000, 7)] {
        let mut input: Vec<Vec<f32>> = (0..ch).map(|c| chords(n, c as u32 + 1).iter().map(|v| v * (1.0 - 0.1 * c as f32)).collect()).collect();
        if ch == 6 {
            input[3] = vec![0.0; n];
        }
        for decoder in decoders(&ff) {
            let Some((mut out, _)) = round_trip(&input, rate, 5.0, decoder, &format!("ch{ch}_{rate}_{decoder}")) else { return };
            if decoder == "vorbis" {
                // ffmpeg's decoder gives WAV order (5.1: FL FR FC LFE BL BR); the stream is in Vorbis
                // order (L C R Ls Rs LFE), which libvorbis passes through
                let wav_to_vorbis: &[usize] = match ch {
                    3 => &[0, 2, 1],
                    6 => &[0, 2, 1, 4, 5, 3],
                    7 => &[0, 2, 1, 5, 6, 4, 3],
                    _ => &[0, 1],
                };
                if ch > 2 {
                    out = wav_to_vorbis.iter().map(|&i| out[i].clone()).collect();
                }
            }
            assert_eq!(out.len(), ch);
            for (c, (a, b)) in input.iter().zip(&out).enumerate() {
                assert_eq!(b.len(), n, "{ch} ch {rate} Hz {decoder}: channel {c} length");
                if a.iter().all(|&v| v == 0.0) {
                    assert!(b.iter().all(|v| v.abs() < 1e-6), "silent channel {c}");
                } else {
                    let snr = snr_db(&a[4096..], &b[4096..]);
                    assert!(snr > 8.0, "{ch} ch {rate} Hz {decoder} channel {c}: {snr:.1} dB");
                }
            }
        }
    }
}

/// Every partition holds whole sample frames (libvorbis decodes each one from channel 0), splits
/// the interleaved residue evenly and fits every book's dimension.
#[test]
fn partitions_hold_whole_frames() {
    for ch in 1..=8 {
        let p = partition_size(ch);
        assert_eq!(p % ch, 0, "{ch} channels");
        assert_eq!((ch * BLOCK / 2) % p, 0, "{ch} channels");
        assert_eq!(p % 4, 0, "{ch} channels");
    }
}
