//! `craft_audio::decode` routes each container and codec below to the home-grown demuxers and
//! decoders, or to symphonia (FLAC): each pair decodes, sample-aligned, to its presentation length
//! (edit lists, Matroska `CodecDelay` and `DiscardPadding`, Ogg Opus pre-skip and end trimming
//! applied), within an SNR of ffmpeg's decode; truncated and corrupted copies never panic. Fixtures
//! are written by ffmpeg (skipped without it).

use craft_audio::{ChannelLayout, CodecId, Container, decode, probe};
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
    container: Container,
    codec: CodecId,
    /// The speakers of the decoded channels (WAV order).
    layout: ChannelLayout,
    channels: usize,
    /// Presented frames, or `None` for a raw stream (as many as ffmpeg decodes).
    frames: Option<usize>,
    /// Minimum SNR against ffmpeg (AC-3 is limited by decoder-specific dither, §7.3.4).
    min_snr: f64,
}

const CASES: &[Case] = &[
    Case {
        name: "flac.flac",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "flac", "-sample_fmt", "s16"],
        container: Container::Flac,
        codec: CodecId::Flac,
        layout: ChannelLayout::STEREO,
        channels: 2,
        frames: Some(240_000),
        min_snr: 140.0,
    },
    Case {
        name: "aac.aac",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "aac", "-b:a", "128k", "-aac_pns", "0"],
        container: Container::Adts,
        codec: CodecId::Aac,
        layout: ChannelLayout::STEREO,
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
        container: Container::Mp4,
        codec: CodecId::Aac,
        layout: ChannelLayout::STEREO,
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
        container: Container::Mp4,
        codec: CodecId::Aac,
        layout: ChannelLayout::SURROUND_5_1_SIDE,
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
        container: Container::Matroska,
        codec: CodecId::Aac,
        layout: ChannelLayout::STEREO,
        channels: 2,
        frames: Some(240_000),
        min_snr: 120.0,
    },
    Case {
        name: "opus.opus",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "libopus", "-b:a", "128k"],
        container: Container::Ogg,
        codec: CodecId::Opus,
        layout: ChannelLayout::STEREO,
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
        container: Container::Ogg,
        codec: CodecId::Opus,
        layout: ChannelLayout::SURROUND_5_1,
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
        container: Container::WebM,
        codec: CodecId::Opus,
        layout: ChannelLayout::STEREO,
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
        container: Container::Mp4,
        codec: CodecId::Opus,
        layout: ChannelLayout::STEREO,
        channels: 2,
        frames: Some(240_000),
        min_snr: 120.0,
    },
    Case {
        name: "ac3.ac3",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "ac3", "-b:a", "192k"],
        container: Container::Ac3,
        codec: CodecId::Ac3,
        layout: ChannelLayout::STEREO,
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
        container: Container::Ac3,
        codec: CodecId::Ac3,
        layout: ChannelLayout::SURROUND_5_1_SIDE,
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
        container: Container::Ac3,
        codec: CodecId::Eac3,
        layout: ChannelLayout::STEREO,
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
        container: Container::Mp4,
        codec: CodecId::Ac3,
        layout: ChannelLayout::STEREO,
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
        container: Container::Matroska,
        codec: CodecId::Eac3,
        layout: ChannelLayout::SURROUND_5_1_SIDE,
        channels: 6,
        frames: Some(240_000),
        min_snr: 25.0,
    },
    Case {
        name: "mp2.mp2",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "mp2"],
        container: Container::MpegAudio,
        codec: CodecId::Mp2,
        layout: ChannelLayout::STEREO,
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
        container: Container::MpegAudio,
        codec: CodecId::Mp2,
        layout: ChannelLayout::MONO,
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
        container: Container::Matroska,
        codec: CodecId::Mp2,
        layout: ChannelLayout::STEREO,
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
        container: Container::Mp4,
        codec: CodecId::Mp2,
        layout: ChannelLayout::STEREO,
        channels: 2,
        frames: Some(240_000),
        min_snr: 80.0,
    },
    Case {
        name: "pcm-s24be.mov",
        surround: false,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "pcm_s24be"],
        container: Container::QuickTime,
        codec: CodecId::Pcm,
        layout: ChannelLayout::STEREO,
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
        container: Container::Matroska,
        codec: CodecId::Pcm,
        layout: ChannelLayout::STEREO,
        channels: 2,
        frames: Some(240_000),
        min_snr: 140.0,
    },
];

fn run(ff: &Path, args: &[&str]) -> bool {
    Command::new(ff).args(["-v", "error", "-y"]).args(args).output().is_ok_and(|o| o.status.success())
}

/// The fixture for `c`, written by ffmpeg (cached), or `None` when this ffmpeg lacks an encoder
/// (a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
fn fixture(ff: &Path, c: &Case) -> Option<PathBuf> {
    let out = crate::testkit::fixtures_dir("routing").join(c.name);
    let source = match (c.mono, c.surround) {
        (true, _) => MONO.to_string(),
        (_, true) => format!("{STEREO}{SURROUND_EXTRA}"),
        _ => STEREO.to_string(),
    };
    let input = format!("{source}:s={}", c.rate);
    let made = crate::testkit::fixtures::generate(&out, |tmp| {
        let tmp = tmp.to_string_lossy();
        let mut args = vec!["-f", "lavfi", "-i", &input, "-t", "5"];
        args.extend_from_slice(c.args);
        args.push(&tmp);
        run(ff, &args)
    });
    assert!(made.is_some() || !craft_media_testkit::oracles_required(), "{}: ffmpeg could not write this fixture", c.name);
    made
}

/// ffmpeg's decode of `file` as interleaved f32.
fn reference(ff: &Path, file: &Path) -> Vec<f32> {
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(file).args(["-f", "f32le", "-c:a", "pcm_f32le", "-"]).output().unwrap();
    assert!(out.status.success(), "ffmpeg decode of {}", file.display());
    out.stdout.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// The major version of this ffmpeg, or `None` for one that does not say (a git build).
fn ffmpeg_major(ff: &Path) -> Option<u32> {
    let out = Command::new(ff).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = text.strip_prefix("ffmpeg version ")?.trim_start_matches('n');
    version.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// How many frames of priming ffmpeg before 9 decodes as audio at the start of `name`, which
/// `decode` drops; ffmpeg 9 drops them too.
fn reference_lead(name: &str) -> usize {
    match name {
        "aac.mkv" => 1024,
        "eac3-5.1.mkv" => 256,
        "mp2.mkv" => 481,
        _ => 0,
    }
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

/// How many frames short of the input ffmpeg before 9 ends `name`: its lengths for these tracks
/// leave out about the encoder's delay. The decoder presents what the file declares.
fn early_end(name: &str) -> usize {
    match name {
        "ac3.mp4" => 288,
        "eac3-5.1.mkv" => 240,
        "mp2.mkv" => 480,
        "mp2.mp4" => 528,
        _ => 0,
    }
}

#[test]
fn every_container_and_codec_decodes_home_grown() {
    let ff = require_ffmpeg!();
    let major = ffmpeg_major(&ff);
    for c in CASES {
        let Some(path) = fixture(&ff, c) else {
            eprintln!("SKIPPED {}: this ffmpeg cannot write it", c.name);
            continue;
        };
        let bytes = std::fs::read(&path).unwrap();
        let (info, buf) = decode(&bytes, None).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let reference = reference(&ff, &path);
        assert_eq!((info.container, info.codec, info.sample_rate, buf.channels.len()), (c.container, c.codec, c.rate, c.channels), "{}", c.name);
        assert_eq!(info.layout, c.layout, "{}: layout", c.name);
        let expected = c.frames.unwrap_or(reference.len() / c.channels);
        let frames = buf.frames();
        assert!(frames == expected || frames == expected - early_end(c.name), "{}: presented frames: {frames} of {expected}", c.name);
        // One alignment, so audio that keeps the priming, or loses more than it, cannot pass.
        let lead = reference_lead(c.name) * c.channels;
        let s = match major {
            Some(9..) => snr(&buf.channels, &reference),
            Some(_) => snr(&buf.channels, &reference[lead..]),
            None => snr(&buf.channels, &reference).max(snr(&buf.channels, &reference[lead..])),
        };
        assert!(s >= c.min_snr, "{}: SNR {s:.1} dB < {}", c.name, c.min_snr);
        let p = probe(&bytes, None).unwrap();
        assert_eq!(
            (p.container, p.codec, p.sample_rate, usize::from(p.channels), p.layout),
            (c.container, c.codec, c.rate, c.channels, c.layout),
            "{}: probe",
            c.name
        );
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

/// SNR (dB) of `ours` against `reference`, both planar, over the frames both hold.
fn snr_planar(ours: &[Vec<f32>], reference: &[Vec<f32>]) -> f64 {
    let (mut s, mut e) = (0f64, 0f64);
    for (o, r) in ours.iter().zip(reference) {
        for (&a, &b) in o.iter().zip(r) {
            s += f64::from(b) * f64::from(b);
            e += f64::from(a - b) * f64::from(a - b);
        }
    }
    if e == 0.0 { f64::INFINITY } else { 10.0 * (s / e).log10() }
}

/// The streaming decoder gives the same audio as `decode`, block by block, and a seek lands on the
/// exact frame: what follows matches `decode` from there, bit for bit.
#[test]
fn streaming_and_seeking_match_the_whole_file_decode() {
    use craft_audio::{DecodeOptions, Decoder};
    let ff = require_ffmpeg!();
    for c in CASES {
        let Some(path) = fixture(&ff, c) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        let (_, whole) = decode(&bytes, None).unwrap();
        let mut d = Decoder::from_bytes(&bytes, None, &DecodeOptions::default()).unwrap();
        let mut streamed: Vec<Vec<f32>> = vec![Vec::new(); whole.channels.len()];
        while let Some(b) = d.next_block().unwrap() {
            for (s, ch) in streamed.iter_mut().zip(b.channels) {
                s.extend(ch);
            }
        }
        assert_eq!(streamed, whole.channels, "{}: streamed blocks", c.name);

        for target in [1, whole.frames() / 3, whole.frames() - 4800] {
            assert_eq!(d.seek(target as u64).unwrap(), target as u64, "{}", c.name);
            let mut after: Vec<Vec<f32>> = vec![Vec::new(); whole.channels.len()];
            while let Some(b) = d.next_block().unwrap() {
                for (s, ch) in after.iter_mut().zip(b.channels) {
                    s.extend(ch);
                }
            }
            assert_eq!(after[0].len(), whole.frames() - target, "{}: frames after a seek to {target}", c.name);
            let expected: Vec<Vec<f32>> = whole.channels.iter().map(|ch| ch[target..].to_vec()).collect();
            let s = snr_planar(&after, &expected);
            // Opus rebuilds its predictor state from the pre-roll. The rest are exact, AC-3 / E-AC-3
            // too: their dither is seeded from each syncframe, and the pre-roll frame restores the
            // overlap.
            let bound = if c.codec == CodecId::Opus { c.min_snr.min(25.0) } else { f64::INFINITY };
            assert!(s >= bound, "{}: SNR {s:.1} dB after a seek to {target}", c.name);
        }
    }
}

/// `TrackDecoder` on the packets craft-media's demuxer reads gives the audio `decode` presents:
/// the same samples once the container's priming is skipped.
#[test]
fn track_decoder_on_demuxed_packets_matches_decode() {
    use craft_audio::TrackDecoder;
    use craft_media::isobmff;
    let ff = require_ffmpeg!();
    for name in ["aac.m4a", "aac-5.1.m4a", "opus.mp4", "mp2.mp4", "pcm-s24be.mov"] {
        let c = CASES.iter().find(|c| c.name == name).unwrap();
        let Some(path) = fixture(&ff, c) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        let (info, whole) = decode(&bytes, None).unwrap();
        let file = isobmff::open(bytes.as_slice()).unwrap();
        let track = file.track_of_kind(isobmff::TrackKind::Audio).unwrap();
        let t = &file.tracks[track];
        let params = t.codec_params();
        let packets: Vec<Vec<u8>> = (0..t.samples.len()).map(|i| file.read_sample(bytes.as_slice(), track, i).unwrap()).collect();
        let lookahead: Vec<&[u8]> = packets.iter().take(8).map(Vec::as_slice).collect();
        let mut dec = TrackDecoder::new(&params, &lookahead).unwrap();
        assert_eq!((dec.codec(), dec.sample_rate(), dec.layout()), (info.codec, info.sample_rate, info.layout), "{name}");
        let mut out: Vec<Vec<f32>> = vec![Vec::new(); usize::from(dec.channels())];
        for p in &packets {
            for (o, ch) in out.iter_mut().zip(dec.decode(p).unwrap().channels) {
                o.extend(ch);
            }
        }
        // The edit list's media time is the priming `decode` skips.
        let skip = t.edits.first().map_or(0, |e| (e.media_time.max(0) as u64 * u64::from(info.sample_rate) / u64::from(t.timescale)) as usize);
        let presented: Vec<Vec<f32>> = out.iter().map(|ch| ch[skip..skip + whole.frames()].to_vec()).collect();
        assert_eq!(presented, whole.channels, "{name}");
    }
}

/// `tracks` lists every audio track, `DecodeOptions::track` picks one, and `max_samples` refuses
/// a longer decode.
#[test]
fn track_choice_and_the_sample_limit() {
    use craft_audio::{AudioError, DecodeOptions, decode_with, tracks};
    let ff = require_ffmpeg!();
    for ext in ["mkv", "mp4"] {
        let out = crate::testkit::fixtures_dir("routing").join(format!("two-tracks.{ext}"));
        let Some(path) = crate::testkit::fixtures::generate(&out, |tmp| {
            let tmp = tmp.to_string_lossy();
            run(
                &ff,
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=f=440:r=48000:d=2",
                    "-f",
                    "lavfi",
                    "-i",
                    "aevalsrc=0.3*sin(2*PI*660*t)|0.3*sin(2*PI*880*t):s=44100:d=2",
                    "-map",
                    "0:a",
                    "-map",
                    "1:a",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "96k",
                    &tmp,
                ],
            )
        }) else {
            continue;
        };
        let bytes = std::fs::read(&path).unwrap();
        let list = tracks(&bytes, None).unwrap();
        assert_eq!(list.len(), 2, "{ext}");
        assert_eq!((list[0].codec, list[0].sample_rate, list[0].channels), (CodecId::Aac, Some(48_000), Some(1)), "{ext}");
        assert_eq!((list[1].sample_rate, list[1].channels), (Some(44_100), Some(2)), "{ext}");
        let (first, _) = decode_with(&bytes, None, &DecodeOptions::default()).unwrap();
        let (second, buf) = decode_with(&bytes, None, &DecodeOptions::default().with_track(1)).unwrap();
        assert_eq!((first.channels, first.sample_rate), (1, 48_000), "{ext}");
        assert_eq!((second.channels, second.sample_rate, buf.frames()), (2, 44_100, 88_200), "{ext}");
        let limited = decode_with(&bytes, None, &DecodeOptions::default().with_max_samples(1000));
        assert!(matches!(limited, Err(AudioError::TooLarge(_))), "{ext}");
        // Refused from the track list, without reading the file through the fallback.
        let missing = decode_with(&bytes, None, &DecodeOptions::default().with_track(2));
        assert!(matches!(&missing, Err(AudioError::Unsupported(m)) if m.contains("no audio track 2")), "{ext}: {missing:?}");
    }
}

/// `Decoder::open` reads from a file without loading it.
#[test]
fn decoder_reads_from_a_file() {
    use craft_audio::{DecodeOptions, Decoder};
    let ff = require_ffmpeg!();
    let c = CASES.iter().find(|c| c.name == "aac.m4a").unwrap();
    let Some(path) = fixture(&ff, c) else { return };
    let (_, whole) = decode(&std::fs::read(&path).unwrap(), None).unwrap();
    let mut d = Decoder::open(std::fs::File::open(&path).unwrap(), None, &DecodeOptions::default()).unwrap();
    assert!(d.length_known());
    assert_eq!(d.info().frames, whole.frames() as u64);
    let mut frames = 0;
    while let Some(b) = d.next_block().unwrap() {
        frames += b.frames();
    }
    assert_eq!(frames, whole.frames());
}

/// `fixture`'s AAC packets in a Matroska file with no block durations, like ffmpeg 6's,
/// and a `DURATION` tag for the track when `tag` is given.
fn aac_mkv_without_durations(bytes: &[u8], tag: Option<&str>) -> Vec<u8> {
    use craft_media::matroska::{Demuxer, LacingMode, MkvWriter, MuxOptions, TrackKind, TrackSpec};
    let mut demux = Demuxer::from_slice(bytes).unwrap();
    let track = demux.file().tracks[0].clone();
    let mut spec = TrackSpec::new(TrackKind::Audio, track.codec_id.clone());
    spec.codec_private = track.codec_private.clone();
    spec.audio = Some((48_000.0, 2, None));
    spec.codec_delay_ns = track.codec_delay_ns;
    let opts = MuxOptions { lacing: LacingMode::None, ..MuxOptions::default() };
    let mut w = MkvWriter::new(std::io::Cursor::new(Vec::new()), vec![spec], opts).unwrap();
    while let Some(p) = demux.next_packet().unwrap() {
        w.write_frame(0, p.pts_ns, true, &p.data, None).unwrap();
    }
    let mut out = w.finish().unwrap().into_inner();
    let Some(tag) = tag else { return out };

    fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
        assert!(body.len() < 127);
        [id, &[0x80 | body.len() as u8], body].concat()
    }
    let uid = Demuxer::from_slice(&out).unwrap().file().tracks[0].uid.to_be_bytes();
    let targets = el(&[0x63, 0xC0], &el(&[0x63, 0xC5], &uid));
    let simple = el(&[0x67, 0xC8], &[el(&[0x45, 0xA3], b"DURATION"), el(&[0x44, 0x87], tag.as_bytes())].concat());
    let tags = el(&[0x12, 0x54, 0xC3, 0x67], &el(&[0x73, 0x73], &[targets, simple].concat()));
    out.extend_from_slice(&tags);
    // The Segment's 8-byte size grows by the tags.
    let at = out.windows(4).position(|w| w == [0x18, 0x53, 0x80, 0x67]).unwrap() + 4;
    let size = u64::from_be_bytes(out[at..at + 8].try_into().unwrap()) + tags.len() as u64;
    out[at..at + 8].copy_from_slice(&size.to_be_bytes());
    out
}

/// A track with no block durations ends where its `DURATION` tag says, when that is inside the
/// last packet. Otherwise all the file's audio is presented.
#[test]
fn matroska_aac_without_block_durations_ends_at_its_duration_tag() {
    let ff = require_ffmpeg!();
    let case = CASES.iter().find(|c| c.name == "aac.mkv").unwrap();
    let Some(path) = fixture(&ff, case) else { return };
    let bytes = std::fs::read(path).unwrap();
    let frames = |tag| {
        let file = aac_mkv_without_durations(&bytes, tag);
        let (info, buf) = decode(&file, None).unwrap();
        assert_eq!(probe(&file, None).unwrap().frames, info.frames, "{tag:?}");
        buf.frames()
    };
    // 5 s, plus the 21 ms of priming the stored timestamps count
    assert_eq!(frames(Some("00:00:05.021000000")), 240_000);
    // 236 packets less the priming
    let all = 236 * 1024 - 1024;
    assert_eq!(frames(None), all);
    assert_eq!(frames(Some("00:00:03.000000000")), all, "ends before the last packet");
    assert_eq!(frames(Some("00:00:09.000000000")), all, "ends after the last packet");
    assert_eq!(frames(Some("not a duration")), all);
}

/// ADTS with channel configuration 0 takes its layout from the program config element in the stream.
#[test]
fn adts_with_a_program_config_element_decodes() {
    let ff = require_ffmpeg!();
    let c = Case {
        name: "aac-pce.aac",
        surround: true,
        mono: false,
        rate: 48_000,
        args: &["-c:a", "aac", "-b:a", "384k", "-aac_pns", "0", "-aac_pce", "1"],
        container: Container::Adts,
        codec: CodecId::Aac,
        layout: ChannelLayout::unordered(6),
        channels: 6,
        frames: None,
        min_snr: 100.0,
    };
    let Some(path) = fixture(&ff, &c) else { return };
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes[3] >> 6 | (bytes[2] & 1) << 2, 0, "the fixture signals its layout in a PCE");
    let (info, buf) = decode(&bytes, None).unwrap();
    assert_eq!((info.container, info.codec, buf.channels.len()), (c.container, c.codec, c.channels));
    // A program config element that lists a centre, a front pair and then the elements of 5.1 has
    // 5.1's speakers, in WAV order like ffmpeg's decode; any other list keeps the decoded order,
    // which ffmpeg versions write differently.
    let ordered = info.layout.is_ordered();
    assert_eq!(info.layout, if ordered { ChannelLayout::SURROUND_5_1_SIDE } else { c.layout });
    assert_eq!(buf.frames(), reference(&ff, &path).len() / c.channels);
    // Each of our channels must match a different one of ffmpeg's: the same one when the layout is
    // ordered.
    let refr = reference(&ff, &path);
    let mut matched = Vec::new();
    for plane in &buf.channels {
        let (best, s) = (0..c.channels)
            .map(|r| {
                let other: Vec<f32> = refr.iter().skip(r).step_by(c.channels).copied().collect();
                (r, snr(std::slice::from_ref(plane), &other))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        assert!(s >= c.min_snr, "channel {} matches ffmpeg's {best} at only {s:.1} dB", matched.len());
        matched.push(best);
    }
    if !ordered {
        matched.sort_unstable();
    }
    assert_eq!(matched, (0..c.channels).collect::<Vec<_>>());
}

/// `probe` and `decode` agree on an MP4's length when no single edit gives it: the samples'
/// durations end the track. ffmpeg 9 does the same, but earlier versions present the whole last
/// packet, so the length is not compared with its decode.
#[test]
fn mp4_without_an_edit_list_ends_with_its_sample_durations() {
    let ff = require_ffmpeg!();
    let c = Case {
        name: "aac-no-edits.m4a",
        surround: false,
        mono: false,
        rate: 44_100,
        args: &["-c:a", "aac", "-b:a", "128k", "-aac_pns", "0", "-use_editlist", "0"],
        container: Container::Mp4,
        codec: CodecId::Aac,
        layout: ChannelLayout::STEREO,
        channels: 2,
        frames: None,
        min_snr: 100.0,
    };
    let Some(path) = fixture(&ff, &c) else { return };
    let bytes = std::fs::read(&path).unwrap();
    let (info, buf) = decode(&bytes, None).unwrap();
    assert_eq!(probe(&bytes, None).unwrap().frames, buf.frames() as u64);
    assert_eq!(info.frames, buf.frames() as u64);
    // 5 s, and 1024 frames of priming with no edit list to skip them.
    assert_eq!(buf.frames(), 5 * 44_100 + 1024);
}

/// The big-endian `u32` at each of `offsets` after the first `box_name` in `file`, changed by `edit`.
fn patched(file: &[u8], box_name: &[u8; 4], offsets: &[usize], edit: impl Fn(u32) -> u32) -> Vec<u8> {
    let mut out = file.to_vec();
    let at = file.windows(4).position(|w| w == box_name).unwrap();
    for &o in offsets {
        let p = at + o;
        let v: [u8; 4] = out[p..p + 4].try_into().unwrap();
        out[p..p + 4].copy_from_slice(&edit(u32::from_be_bytes(v)).to_be_bytes());
    }
    out
}

/// Wrong sample durations or edits don't cut audio or disagree with `probe`: durations only trim the
/// last packet, and an edit can't outlast the media.
#[test]
fn mp4_with_wrong_durations_or_edits_presents_its_audio() {
    let ff = require_ffmpeg!();
    let c = CASES.iter().find(|c| c.name == "aac.m4a").unwrap();
    let Some(path) = fixture(&ff, c) else { return };
    let bytes = std::fs::read(path).unwrap();
    let (_, whole) = decode(&bytes, None).unwrap();
    // From the name's start: the name (4), version and flags (4), entries (4), then stts's first
    // run's count (4) and delta, or elst v0's first duration.
    let stts_delta = 4 + 4 + 4 + 4;
    let elst_duration = 4 + 4 + 4;
    for (name, file) in [
        ("durations halved", patched(&bytes, b"stts", &[stts_delta], |v| v / 2)),
        ("durations of one", patched(&bytes, b"stts", &[stts_delta], |_| 1)),
        ("durations doubled", patched(&bytes, b"stts", &[stts_delta], |v| v * 2)),
        ("durations of zero", patched(&bytes, b"stts", &[stts_delta], |_| 0)),
        ("last delta longer than a packet", patched(&bytes, b"stts", &[stts_delta + 8], |_| 1500)),
    ] {
        assert_ne!(file, bytes, "{name}");
        let (info, buf) = decode(&file, None).unwrap();
        assert!(buf.frames().abs_diff(whole.frames()) <= 1024, "{name}: {} frames", buf.frames());
        assert_eq!((probe(&file, None).unwrap().frames, info.frames), (buf.frames() as u64, buf.frames() as u64), "{name}");
        // A seek lands on the same audio: a packet's place comes from its index, not the durations.
        let mut d = craft_audio::Decoder::from_bytes(&file, None, &craft_audio::DecodeOptions::default()).unwrap();
        assert_eq!(d.seek(50_000).unwrap(), 50_000, "{name}");
        let block = d.next_block().unwrap().unwrap();
        for (a, b) in block.channels[0].iter().zip(&buf.channels[0][50_000..]) {
            assert!((a - b).abs() < 1e-4, "{name}: seek");
        }
    }
    let file = patched(&bytes, b"elst", &[elst_duration], |v| v * 2);
    let (info, buf) = decode(&file, None).unwrap();
    assert!(buf.frames() < whole.frames() + 2048, "edit doubled: {} frames", buf.frames());
    assert_eq!((probe(&file, None).unwrap().frames, info.frames), (buf.frames() as u64, buf.frames() as u64), "edit doubled");
}

/// An empty edit before the media edit (QuickTime's and GPAC's delayed audio, ffmpeg's
/// `-itsoffset`) still has the media edit applied: its priming skip and its end, as ffmpeg's
/// decode does. ffmpeg writes such a media edit with no skip, so the test sets one.
#[test]
fn mp4_with_an_empty_edit_before_its_media_edit_applies_the_media_edit() {
    let ff = require_ffmpeg!();
    let dir = crate::testkit::fixtures_dir("routing");
    let out = dir.join("aac-itsoffset.m4a");
    let input = format!("{MONO}:s=44100");
    let made = crate::testkit::fixtures::generate(&out, |tmp| {
        let tmp = tmp.to_string_lossy();
        run(ff.as_path(), &["-itsoffset", "0.5", "-f", "lavfi", "-i", &input, "-t", "5", "-c:a", "aac", "-aac_pns", "0", &tmp])
    });
    assert!(made.is_some() || !craft_media_testkit::oracles_required(), "ffmpeg could not write aac-itsoffset.m4a");
    let Some(path) = made else { return };
    let bytes = std::fs::read(&path).unwrap();
    // elst v0 from the name's start: the name (4), version and flags (4), entries (4), then 12
    // bytes per edit: duration, media time, rate.
    let elst = bytes.windows(4).position(|w| w == b"elst").unwrap();
    assert_eq!((bytes[elst + 4], u32::from_be_bytes(bytes[elst + 8..elst + 12].try_into().unwrap())), (0, 2), "an empty edit and a media edit");
    assert_eq!(i32::from_be_bytes(bytes[elst + 12 + 4..elst + 12 + 8].try_into().unwrap()), -1, "the first edit is empty");
    let media_time = elst + 12 + 12 + 4;
    let mut file = bytes.clone();
    file[media_time..media_time + 4].copy_from_slice(&1024i32.to_be_bytes());
    let patched = dir.join("aac-itsoffset-priming.m4a");
    std::fs::write(&patched, &file).unwrap();
    let reference = reference(&ff, &patched);
    let (info, buf) = decode(&file, None).unwrap();
    assert_eq!(buf.frames(), reference.len(), "presented frames");
    assert_eq!((probe(&file, None).unwrap().frames, info.frames), (buf.frames() as u64, buf.frames() as u64));
    let s = snr(&buf.channels, &reference);
    assert!(s >= 100.0, "SNR {s:.1} dB: the priming skip is not applied");
}
