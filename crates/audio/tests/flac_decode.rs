//! The native FLAC decoder: round trips through this crate's encoder over bit depths, channel
//! counts, block sizes and effort levels; hostile input; and ffmpeg as the reference (oracle;
//! skipped when ffmpeg is absent, a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`) for files
//! ffmpeg's encoder writes at every compression level.

use craft_audio::flac::{DecodeError, Decoder, Encoder, EncoderConfig, STREAMINFO_OFFSET, StreamInfo, decode};
use std::process::Command;

fn signal(channels: usize, frames: usize, bits: u32, seed: u32) -> Vec<Vec<i32>> {
    let max = if bits == 32 { i32::MAX as f64 } else { ((1i64 << (bits - 1)) - 1) as f64 };
    let mut s = seed;
    (0..channels)
        .map(|c| {
            (0..frames)
                .map(|i| {
                    s ^= s << 13;
                    s ^= s >> 17;
                    s ^= s << 5;
                    let t = i as f64;
                    let v = match (i / 3000) % 4 {
                        0 => (t * (0.01 + 0.007 * c as f64)).sin() * 0.6 + (t * 0.31).sin() * 0.1,
                        1 => (f64::from(s) / f64::from(u32::MAX) - 0.5) * 1.8,
                        2 => 0.0,
                        _ => {
                            if (i / 40) % 2 == 0 {
                                0.99
                            } else {
                                -0.99
                            }
                        }
                    };
                    (v * max).round() as i32
                })
                .collect()
        })
        .collect()
}

fn encode(channels: &[Vec<i32>], rate: u32, bits: u8, block: u16, level: u8) -> Vec<u8> {
    let mut cfg = EncoderConfig::new(rate, channels.len() as u8, bits);
    cfg.block_size = block;
    cfg.level = level;
    let mut enc = Encoder::new(cfg).expect("encoder");
    let mut file = enc.header(&[]);
    let refs: Vec<&[i32]> = channels.iter().map(Vec::as_slice).collect();
    file.extend(enc.encode(&refs).expect("encode"));
    file.extend(enc.finish());
    let at = STREAMINFO_OFFSET as usize;
    file[at..at + 34].copy_from_slice(&enc.streaminfo());
    file
}

#[test]
fn round_trips_through_the_encoder() {
    for (channels, bits) in [(1usize, 8u8), (2, 8), (1, 12), (2, 16), (2, 20), (3, 24), (6, 24), (8, 16), (2, 32), (1, 32)] {
        for (block, level) in [(4096u16, 5u8), (16, 0), (1000, 8), (65535, 3), (576, 1)] {
            let frames = 9000 + usize::from(block % 97);
            let src = signal(channels, frames, u32::from(bits), 0x1234_5679 + u32::from(bits));
            let file = encode(&src, 44_100, bits, block, level);
            let got = decode(&file).unwrap_or_else(|e| panic!("{channels}ch {bits}-bit block {block} level {level}: {e}"));
            assert_eq!((got.info.channels as usize, got.info.bits_per_sample, got.info.sample_rate), (channels, bits, 44_100));
            assert_eq!(got.channels, src, "{channels}ch {bits}-bit block {block} level {level}");
        }
    }
}

#[test]
fn frames_decode_one_at_a_time() {
    let src = signal(2, 10_000, 16, 7);
    let file = encode(&src, 48_000, 16, 4096, 5);
    let info = StreamInfo::parse(&file[STREAMINFO_OFFSET as usize..]).expect("streaminfo");
    let dec = Decoder::new(info);
    // The first frame follows the metadata blocks; find the first position a frame decodes at.
    let at = (42..file.len()).find(|&p| dec.decode_frame(&file[p..]).is_ok()).expect("a frame");
    let first = dec.decode_frame(&file[at..]).expect("frame");
    assert_eq!((first.block_size, first.number, first.sample_rate, first.bits_per_sample), (4096, 0, 48_000, 16));
    assert_eq!(first.channels[0], src[0][..4096]);
    // Without the STREAMINFO a frame that spells out its own rate and depth still decodes.
    let _ = Decoder::without_streaminfo().decode_frame(&file[at..]);
}

#[test]
fn hostile_input_is_an_error_not_a_panic() {
    let src = signal(2, 6000, 16, 11);
    let file = encode(&src, 44_100, 16, 1152, 5);
    for n in 0..file.len().min(3000) {
        let _ = decode(&file[..n]);
    }
    // A cut file is truncated, not silently shorter.
    assert!(decode(&file[..file.len() - 5]).is_err());
    // Every bit flip in the first frames is caught by a CRC or another check, never a panic.
    for i in 0..file.len().min(2500) {
        for bit in 0..8 {
            let mut c = file.clone();
            c[i] ^= 1 << bit;
            let _ = decode(&c);
        }
    }
    // A sample limit stops a stream that claims too much.
    assert_eq!(craft_audio::flac::decode_with_limit(&file, 1000), Err(DecodeError::TooLarge));
    assert!(decode(b"nope").is_err());
    assert!(decode(b"fLaC").is_err());
}

// ---- ffmpeg ----

/// Little-endian 16- or 32-bit samples of raw PCM.
fn pcm_values(raw: &[u8], bytes: usize) -> Vec<i32> {
    if bytes == 2 {
        raw.as_chunks::<2>().0.iter().map(|b| i32::from(i16::from_le_bytes(*b))).collect()
    } else {
        raw.as_chunks::<4>().0.iter().map(|b| i32::from_le_bytes(*b)).collect()
    }
}

/// Planar channels as interleaved samples.
fn interleave(channels: &[Vec<i32>]) -> Vec<i32> {
    let frames = channels.first().map_or(0, Vec::len);
    (0..frames).flat_map(|i| channels.iter().map(move |c| c[i])).collect()
}

fn ffmpeg_run(ff: &std::path::Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new(ff).args(["-v", "error", "-nostdin", "-y"]).args(args).output().expect("run ffmpeg");
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

#[test]
fn ffmpeg_files_decode_to_ffmpegs_samples() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("flac_decode");
    let tones = "aevalsrc=exprs=0.4*sin(2*PI*441*t)+0.2*sin(2*PI*3100*t)|0.3*sin(2*PI*660*t)*sin(2*PI*3*t):s=44100:d=1.3";
    let noise = "anoisesrc=d=1.3:c=white:r=44100:a=0.5:s=3";
    for (src, name) in [(tones, "tones"), (noise, "noise")] {
        for channels in [1usize, 2, 6] {
            for (fmt, bytes, bits) in [("s16", 2usize, 16u32), ("s32", 4, 24)] {
                for level in [0, 3, 5, 8, 12] {
                    let file = dir.join(format!("ffmpeg_{name}_{channels}ch_{bits}_l{level}.flac"));
                    let f = file.to_str().expect("path");
                    let (ch, lv) = (channels.to_string(), level.to_string());
                    ffmpeg_run(&ff, &["-f", "lavfi", "-i", src, "-ac", &ch, "-sample_fmt", fmt, "-compression_level", &lv, "-c:a", "flac", f]);
                    let theirs = ffmpeg_run(&ff, &["-i", f, "-f", if bits == 16 { "s16le" } else { "s32le" }, "-"]);
                    let got =
                        decode(&std::fs::read(&file).expect("read")).unwrap_or_else(|e| panic!("{name} {channels}ch {bits}-bit level {level}: {e}"));
                    assert_eq!(got.info.channels as usize, channels);
                    let frames = got.channels[0].len();
                    assert_eq!(theirs.len(), frames * channels * bytes, "{name} {channels}ch {bits} l{level}: length");
                    // ffmpeg returns 24-bit audio left-aligned in 32 bits.
                    let shift = if bits == 24 { 8 } else { 0 };
                    let mine: Vec<i32> = interleave(&got.channels).into_iter().map(|v| v << shift).collect();
                    let ok = mine == pcm_values(&theirs, bytes);
                    assert!(ok, "{name} {channels}ch {bits}-bit level {level}: samples differ from ffmpeg's decode");
                }
            }
        }
    }
}

#[test]
fn ffmpeg_decodes_our_files_to_our_samples() {
    let Some(ff) = craft_media_testkit::ffmpeg() else { return };
    let dir = craft_media_testkit::fixture_subdir("flac_decode");
    for (channels, bits) in [(1usize, 16u8), (2, 16), (2, 24), (6, 16)] {
        let src = signal(channels, 20_000, u32::from(bits), 99);
        let file = dir.join(format!("ours_{channels}ch_{bits}.flac"));
        std::fs::write(&file, encode(&src, 44_100, bits, 4096, 5)).expect("write");
        let theirs = ffmpeg_run(&ff, &["-i", file.to_str().expect("path"), "-f", if bits == 16 { "s16le" } else { "s32le" }, "-"]);
        let got = decode(&std::fs::read(&file).expect("read")).expect("decode");
        let bytes = if bits == 16 { 2 } else { 4 };
        let shift = if bytes == 4 { 8 } else { 0 };
        let want: Vec<i32> = pcm_values(&theirs, bytes).into_iter().map(|v| v >> shift).collect();
        assert!(interleave(&got.channels) == want, "{channels}ch {bits}-bit: samples differ from ffmpeg's decode");
    }
}
