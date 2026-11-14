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

#[test]
fn channel_layouts_rates_and_sizes() {
    let cases: [(&str, u8, u8, u32, usize); 12] = [
        ("mono-8bit", 1, 8, 8_000, 3_001),
        ("mono-24-44k1", 1, 24, 44_100, 9_999),
        ("stereo-20bit", 2, 20, 96_000, 10_000),
        ("5.1-24", 6, 24, 48_000, 12_345),
        ("mono-32", 1, 32, 48_000, 7_777),
        ("5.1-32", 6, 32, 96_000, 12_345),
        ("odd-rate", 2, 16, 37_800, 5_000),
        ("rate-khz", 2, 16, 64_000, 5_000),
        ("rate-tens", 2, 24, 352_800, 5_000),
        ("rate-max", 1, 16, 655_350, 5_000),
        ("rate-from-streaminfo", 1, 16, 100_001, 5_000),
        ("tiny", 2, 16, 48_000, 1),
    ];
    for (name, ch, bps, rate, n) in cases {
        let planar: Vec<Vec<i32>> = (0..ch).map(|c| signal(if c == 3 { "silence" } else { "music" }, n, bps, u64::from(c) + 7)).collect();
        if round_trip(name, &planar, EncoderConfig::new(rate, ch, bps), 1000).is_none() {
            return;
        }
    }
}

#[test]
fn levels_and_block_sizes_trade_time_for_size() {
    let planar = vec![signal("music", 48_000, 16, 11), signal("sweep", 48_000, 16, 12)];
    let mut sizes = Vec::new();
    for level in 0..=8 {
        let cfg = EncoderConfig { level, ..EncoderConfig::new(48_000, 2, 16) };
        let Some(size) = round_trip(&format!("level{level}"), &planar, cfg, 48_000) else { return };
        sizes.push(size);
    }
    // linear prediction beats the fastest level by a wide margin, and all beat PCM
    assert!(sizes[8] <= sizes[5] && sizes[5] < sizes[0] * 4 / 5, "{sizes:?}");
    assert!(sizes.iter().all(|&s| s < 48_000 * 4), "{sizes:?}");
    for bs in [16u16, 192, 1152, 4608, 65_535] {
        let cfg = EncoderConfig { block_size: bs, ..EncoderConfig::new(48_000, 2, 16) };
        round_trip(&format!("block{bs}"), &planar, cfg, 777);
    }
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    assert!(Encoder::new(EncoderConfig::new(48_000, 0, 16)).is_err());
    assert!(Encoder::new(EncoderConfig::new(48_000, 9, 16)).is_err());
    assert!(Encoder::new(EncoderConfig::new(48_000, 2, 33)).is_err());
    assert!(Encoder::new(EncoderConfig::new(48_000, 2, 7)).is_err());
    assert!(Encoder::new(EncoderConfig::new(0, 2, 16)).is_err());
    assert!(Encoder::new(EncoderConfig { block_size: 8, ..EncoderConfig::new(48_000, 2, 16) }).is_err());
    let mut enc = Encoder::new(EncoderConfig::new(48_000, 2, 16)).unwrap();
    assert_eq!(enc.encode(&[&[0, 1]]), Err(Error::Channels { expected: 2, got: 1 }));
    assert_eq!(enc.encode(&[&[0, 1], &[0]]), Err(Error::UnequalChannels));
    assert_eq!(enc.encode(&[&[40_000], &[0]]), Err(Error::SampleRange(40_000)));
    assert_eq!(enc.encode(&[&[-32_769], &[0]]), Err(Error::SampleRange(-32_769)));
    // nothing encoded: no frames, and a header that still says "unknown"
    assert!(enc.finish().is_empty());
    assert_eq!(&enc.header(&[])[..4], b"fLaC");
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

/// A layout decoders would not assume for the channel count is recorded as a channel mask comment,
/// which ffmpeg reads: they read 6 channels with side surrounds, and 7 as 6.1 with sides.
#[test]
fn layouts_decoders_would_not_assume_are_recorded() {
    use crate::{AudioBuffer, ChannelLayout, EncodeFormat, EncodeOptions};
    let Some(ffprobe) = common::ffprobe() else { return };
    let cases = [
        ("layout-stereo", 2, None, "stereo"),
        ("layout-7-default", 7, None, "6.1(back)"),
        ("layout-6.1-side", 7, Some(ChannelLayout::SURROUND_6_1), "6.1"),
        ("layout-5.1-side", 6, Some(ChannelLayout::SURROUND_5_1_SIDE), "5.1(side)"),
        ("layout-5.1-back", 6, Some(ChannelLayout::SURROUND_5_1), "5.1"),
        ("layout-6-default", 6, None, "5.1"),
    ];
    for (name, channels, layout, expected) in cases {
        let buf = AudioBuffer::new(48_000, channels, 100);
        let mut opts = EncodeOptions::new(EncodeFormat::Flac);
        opts.layout = layout;
        let file = crate::encode(&buf, &opts).unwrap();
        let path = common::fixture_subdir("flac").join(format!("{name}.flac"));
        std::fs::write(&path, &file).unwrap();
        let out =
            Command::new(&ffprobe).args(["-v", "error", "-show_entries", "stream=channel_layout", "-of", "csv=p=0"]).arg(&path).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected, "{name}");
    }
}

/// FLAC in Ogg reads its layout as a FLAC file does: from the mask comment, or else the one
/// decoders assume for the count.
#[cfg(feature = "symphonia")]
#[test]
fn ogg_flac_reads_the_mask_comment() {
    use crate::ChannelLayout;
    let Some(ff) = ffmpeg() else { return };
    let cases: [(&str, &str, &[&str], u16, u32); 5] = [
        ("ogg-mask", "aevalsrc=0|0|0|0:c=4c:d=0.1:s=48000", &["-metadata", "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0107"], 4, 0x107),
        ("ogg-5.1", "aevalsrc=0|0|0|0|0|0:c=5.1:d=0.1:s=48000", &[], 6, 0x60F),
        ("ogg-stereo", "aevalsrc=0|0:c=stereo:d=0.1:s=48000", &["-metadata", "TITLE=x"], 2, 0x3),
        ("ogg-6.1", "aevalsrc=0|0|0|0|0|0|0:c=6.1:d=0.1:s=48000", &[], 7, 0x70F),
        ("ogg-zero-mask", "aevalsrc=0|0|0|0|0|0:c=5.1:d=0.1:s=48000", &["-metadata", "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0"], 6, 0),
    ];
    for (name, source, extra, channels, mask) in cases {
        let path = common::fixture_subdir("flac").join(format!("{name}.oga"));
        let status = Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", source, "-c:a", "flac", "-f", "ogg"])
            .args(extra)
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success(), "{name}");
        let bytes = std::fs::read(&path).unwrap();
        let expected = ChannelLayout::from_mask_and_channels(mask, channels);
        assert_eq!(crate::probe(&bytes, None).unwrap().layout, expected, "probe {name}");
        assert_eq!(crate::decode(&bytes, None).unwrap().0.layout, expected, "decode {name}");
    }
}

/// `file`, an Ogg file of two streams, with its streams' serial numbers replaced by `serials`, in
/// the order the streams start.
#[cfg(feature = "symphonia")]
fn set_ogg_serials(mut file: Vec<u8>, serials: [u32; 2]) -> Vec<u8> {
    let mut seen = Vec::new();
    let mut at = 0;
    while at < file.len() {
        assert_eq!(&file[at..at + 4], b"OggS");
        let segments = usize::from(file[at + 26]);
        let body: usize = file[at + 27..at + 27 + segments].iter().map(|&n| usize::from(n)).sum();
        let end = at + 27 + segments + body;
        let old: [u8; 4] = file[at + 14..at + 18].try_into().unwrap();
        let stream = seen.iter().position(|s| *s == old).unwrap_or_else(|| {
            seen.push(old);
            seen.len() - 1
        });
        file[at + 14..at + 18].copy_from_slice(&serials[stream].to_le_bytes());
        // The page's checksum covers its serial number, and is computed with its own field zero.
        file[at + 22..at + 26].fill(0);
        let crc = craft_media::ogg::crc32(&file[at..end]);
        file[at + 22..at + 26].copy_from_slice(&crc.to_le_bytes());
        at = end;
    }
    assert_eq!(seen.len(), 2);
    file
}

/// Two streams in an Ogg file written by ffmpeg, with serial numbers in file order or in `reversed`
/// order. ffmpeg picks them at random, and symphonia orders tracks by serial, so the reversed order
/// is the one that tells the two orders apart. `None` after `skip` when this ffmpeg cannot write the
/// file.
#[cfg(feature = "symphonia")]
fn two_ogg_streams(ff: &Path, name: &str, args: &[&str], reversed: bool) -> Option<Vec<u8>> {
    let path = common::fixture_subdir("flac").join(format!("{name}.ogg"));
    let status = Command::new(ff)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "aevalsrc=0|0|0|0:c=4c:d=0.1:s=48000"])
        .args(["-f", "lavfi", "-i", "aevalsrc=0|0|0|0:c=4c:d=0.1:s=44100"])
        .args(["-map", "0", "-map", "1"])
        .args(args)
        .args(["-f", "ogg"])
        .arg(&path)
        .status()
        .unwrap();
    if !status.success() {
        return common::skip(&format!("this ffmpeg cannot write {name}"));
    }
    let serials = if reversed { [2, 1] } else { [1, 2] };
    let bytes = set_ogg_serials(std::fs::read(&path).unwrap(), serials);
    let file = craft_media::ogg::open(bytes.as_slice()).unwrap();
    assert_eq!(file.streams.iter().map(|s| s.serial).collect::<Vec<_>>(), serials, "{name}");
    assert!(file.warnings.is_empty(), "{name}: {:?}", file.warnings);
    Some(bytes)
}

/// A track index means the same stream in `tracks`, `probe` and `decode`: the file's order. Each
/// track has its own mask comment, or else the layout decoders assume for the count.
#[cfg(feature = "symphonia")]
#[test]
fn ogg_flac_tracks_are_in_file_order_with_their_own_mask_comments() {
    use crate::{ChannelLayout, DecodeOptions};
    let Some(ff) = ffmpeg() else { return };
    let args = ["-c:a", "flac", "-metadata:s:a:0", "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0107"];
    for reversed in [false, true] {
        let Some(bytes) = two_ogg_streams(&ff, "ogg-one-mask", &args, reversed) else { return };
        let rates: Vec<_> = crate::tracks(&bytes, None).unwrap().iter().map(|t| t.sample_rate).collect();
        assert_eq!(rates, [Some(48_000), Some(44_100)]);
        for (track, rate, layout) in [(0, 48_000, ChannelLayout::from_mask(0x107)), (1, 44_100, ChannelLayout::default_for(4))] {
            let opts = DecodeOptions::default().with_track(track);
            let info = crate::probe_with(&bytes, None, &opts).unwrap();
            assert_eq!((info.sample_rate, info.layout), (rate, layout), "probe track {track}");
            let info = crate::decode_with(&bytes, None, &opts).unwrap().0;
            assert_eq!((info.sample_rate, info.layout), (rate, layout), "decode track {track}");
        }
        assert_eq!(crate::decode(&bytes, None).unwrap().0.sample_rate, 48_000, "the first track");
    }
}

/// A FLAC stream before a stream symphonia cannot decode is still the first track.
#[cfg(feature = "symphonia")]
#[test]
fn ogg_flac_before_opus_is_the_first_track() {
    use crate::CodecId;
    let Some(ff) = ffmpeg() else { return };
    let args = ["-c:a:0", "flac", "-c:a:1", "libopus", "-ac:a:1", "2"];
    for reversed in [false, true] {
        let Some(bytes) = two_ogg_streams(&ff, "ogg-flac-opus", &args, reversed) else { return };
        assert_eq!(crate::tracks(&bytes, None).unwrap()[0].codec, CodecId::Flac);
        assert_eq!(crate::decode(&bytes, None).unwrap().0.codec, CodecId::Flac);
    }
}

/// The first page of a Speex stream, which symphonia has no reader for.
#[cfg(feature = "symphonia")]
fn speex_ogg_page(serial: u32) -> Vec<u8> {
    let mut packet = b"Speex   ".to_vec();
    packet.resize(80, 0);
    let mut page = b"OggS\0\x02".to_vec(); // the start of a stream
    page.extend_from_slice(&[0; 8]); // granule position
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&[0; 8]); // page number and checksum
    page.extend_from_slice(&[1, packet.len() as u8]);
    page.extend_from_slice(&packet);
    let crc = craft_media::ogg::crc32(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// A stream symphonia has no track for does not stop the default decode from reading the next one,
/// and it keeps its index.
#[cfg(feature = "symphonia")]
#[test]
fn ogg_flac_after_a_stream_with_no_reader_is_the_default_track() {
    use crate::{CodecId, DecodeOptions};
    let Some(ff) = ffmpeg() else { return };
    let args = ["-c:a", "flac"];
    for reversed in [false, true] {
        let Some(flac) = two_ogg_streams(&ff, "ogg-speex-flac", &args, reversed) else { return };
        for speex_serial in [0, 9] {
            let file = [speex_ogg_page(speex_serial), flac.clone()].concat();
            let codecs: Vec<_> = crate::tracks(&file, None).unwrap().iter().map(|t| t.codec).collect();
            assert_eq!(codecs, [CodecId::Speex, CodecId::Flac, CodecId::Flac]);
            let info = crate::decode(&file, None).unwrap().0;
            assert_eq!((info.codec, info.sample_rate), (CodecId::Flac, 48_000), "reversed {reversed}, speex {speex_serial}");
            assert!(crate::decode_with(&file, None, &DecodeOptions::default().with_track(0)).is_err());
            let second = crate::decode_with(&file, None, &DecodeOptions::default().with_track(2)).unwrap().0;
            assert_eq!(second.sample_rate, 44_100, "reversed {reversed}, speex {speex_serial}");
        }
    }
}

/// Bytes before the first page do not lose the file's track order or mask comments.
#[cfg(feature = "symphonia")]
#[test]
fn ogg_flac_after_leading_bytes_keeps_its_track_order() {
    use crate::{ChannelLayout, DecodeOptions};
    let Some(ff) = ffmpeg() else { return };
    let args = ["-c:a", "flac", "-metadata:s:a:0", "WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0107"];
    let Some(bytes) = two_ogg_streams(&ff, "ogg-leading-bytes", &args, true) else { return };
    let file = [vec![0; 64], bytes].concat();
    let info = crate::decode_with(&file, None, &DecodeOptions::default().with_track(0)).unwrap().0;
    assert_eq!((info.sample_rate, info.layout), (48_000, ChannelLayout::from_mask(0x107)));
}

#[test]
fn the_mask_comment_is_written_only_when_decoders_would_assume_otherwise() {
    use super::mask_comment;
    let comment = |m: &str| Some(format!("WAVEFORMATEXTENSIBLE_CHANNEL_MASK={m}"));
    for channels in [1, 2, 3, 4, 8] {
        assert_eq!(mask_comment(channels, None), None, "{channels}");
    }
    assert_eq!(mask_comment(5, None), comment("0x0037"));
    assert_eq!(mask_comment(5, Some(0x607)), None);
    assert_eq!(mask_comment(6, None), comment("0x003F"));
    assert_eq!(mask_comment(6, Some(0x60F)), None);
    assert_eq!(mask_comment(7, None), comment("0x013F"));
    assert_eq!(mask_comment(7, Some(0x70F)), None);
    // no speaker assignment
    assert_eq!(mask_comment(2, Some(0)), comment("0x0000"));
}

/// Left and right at opposite full scale: the side channel needs its extra bit at every depth.
#[test]
fn opposite_polarity_stereo_round_trips() {
    for bps in [16u8, 24] {
        let left = signal("square", 9_000, bps, 1);
        let right: Vec<i32> = left.iter().map(|&v| !v).collect();
        let planar = vec![left, right];
        if round_trip(&format!("opposite-{bps}"), &planar, EncoderConfig::new(48_000, 2, bps), 4096).is_none() {
            return;
        }
    }
}

#[test]
fn streaminfo_states_the_block_and_frame_sizes() {
    // silence in the middle: the smallest frame is neither the first nor the last
    let channel = |seed| [signal("noise", 4096, 16, seed), signal("silence", 4096, 16, seed), signal("noise", 1000, 16, seed)].concat();
    let planar = [channel(21), channel(22)];
    let mut enc = Encoder::new(EncoderConfig::new(48_000, 2, 16)).unwrap();
    let parts: Vec<&[i32]> = planar.iter().map(Vec::as_slice).collect();
    let mut sizes: Vec<usize> = enc.encode_frames(&parts).unwrap().iter().map(Vec::len).collect();
    sizes.push(enc.finish().len());
    let si = enc.streaminfo();
    let u24 = |b: &[u8]| usize::from(b[0]) << 16 | usize::from(b[1]) << 8 | usize::from(b[2]);
    assert_eq!(&si[..4], &[0x10, 0x00, 0x10, 0x00]);
    assert_eq!(u24(&si[4..7]), sizes.iter().copied().min().unwrap());
    assert_eq!(u24(&si[7..10]), sizes.iter().copied().max().unwrap());
}

/// Frame numbers count up from 0; a decoder that seeks depends on them, though one that reads from
/// the start does not.
#[test]
fn frames_are_numbered_in_order() {
    let planar = [signal("music", 4096 * 10, 16, 31)];
    let mut enc = Encoder::new(EncoderConfig::new(48_000, 1, 16)).unwrap();
    let frames = enc.encode_frames(&[&planar[0]]).unwrap();
    let numbers: Vec<u8> = frames.iter().map(|f| f[4]).collect();
    assert_eq!(numbers, (0..10).collect::<Vec<u8>>());
}

/// Two equal channels cost little more than one: the side channel is all zeros.
#[test]
fn stereo_decorrelation_removes_a_duplicated_channel() {
    let x = signal("music", 20_000, 16, 41);
    let mono = encode(std::slice::from_ref(&x), EncoderConfig::new(48_000, 1, 16), 4096).unwrap();
    let stereo = encode(&[x.clone(), x], EncoderConfig::new(48_000, 2, 16), 4096).unwrap();
    assert!(stereo.len() < mono.len() * 11 / 10, "stereo {} mono {}", stereo.len(), mono.len());
}

#[test]
fn the_frame_limit_stops_a_write_before_it_reaches_the_sink() {
    use super::Writer;
    use crate::{AudioError, BitDepth};
    let mut w = Writer::new(std::io::Cursor::new(Vec::new()), 1, 48_000, BitDepth::Int16, false, None).unwrap();
    let header = w.out.get_ref().len();
    w.frames = (1 << 36) - 2;
    assert!(matches!(w.write(&[[0.0f32; 4096]], 4096), Err(AudioError::TooLarge(_))));
    assert_eq!(w.out.get_ref().len(), header);
    // refused, not failed: what fits can still be written
    assert!(w.write(&[[0.0f32; 1]], 1).is_ok());
    assert!(matches!(w.write(&[[0.0f32; 1]], 1), Err(AudioError::TooLarge(_))));
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

/// A click at full scale needs a residual near the limit; a block is not stored verbatim for it.
#[test]
fn a_full_scale_click_at_32_bits_is_still_predicted() {
    let planar = [signal("click", 4_096, 32, 61)];
    let Some(size) = round_trip("click-32", &planar, EncoderConfig::new(48_000, 1, 32), 4096) else { return };
    assert!(size < 4_096 * 4 / 2, "{size}");
}

/// Signals that reach the format's limits, which a decoder refuses when they are passed: a Rice
/// parameter of 31 is the escape code, and an LPC shift above 15 reads as negative.
#[test]
fn rice_parameter_and_lpc_shift_stay_in_range() {
    let mut r = Rng(71);
    let bursts: Vec<i32> =
        (0..9_000).map(|i| if i % 1024 < 128 { if i % 2 == 0 { i32::MAX } else { -i32::MAX } } else { (r.next() * 100.0) as i32 }).collect();
    if round_trip("limit-rice", &[bursts], EncoderConfig::new(48_000, 1, 32), 4096).is_none() {
        return;
    }
    let mut y = 0.0;
    let correlated: Vec<i32> = (0..9_000)
        .map(|_| {
            y = 0.24 * y + r.next() * 8000.0;
            y as i32
        })
        .collect();
    round_trip("limit-shift", &[correlated], EncoderConfig::new(48_000, 1, 16), 4096);
}
