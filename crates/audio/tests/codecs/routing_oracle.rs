//! `craft_audio::decode` routes each container and codec to the home-grown demuxers and
//! decoders: every pair decodes, sample-aligned, to its presentation length (edit lists, Matroska
//! `CodecDelay` and `DiscardPadding`, Ogg Opus pre-skip and end trimming applied), within an SNR
//! of ffmpeg's decode; truncated and corrupted copies never panic. Fixtures are written by ffmpeg
//! (skipped without it).

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
        assert_eq!((info.container, info.codec, info.sample_rate, buf.channels.len()), (c.container, c.codec, c.rate, c.channels), "{}", c.name);
        assert_eq!(info.layout, c.layout, "{}: layout", c.name);
        let expected = c.frames.unwrap_or(reference.len() / c.channels);
        assert_eq!(buf.frames(), expected, "{}: presented frames", c.name);
        let s = snr(&buf.channels, &reference);
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
/// exact frame: what follows matches `decode` from there (bit for bit for the deterministic
/// codecs; within the codec's bound where the decoder state is reconstructed from a pre-roll).
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
            // Opus rebuilds its predictor state from the pre-roll; AC-3 dither is random per decode.
            let bound = if matches!(c.codec, CodecId::Opus | CodecId::Ac3 | CodecId::Eac3) { c.min_snr.min(25.0) } else { f64::INFINITY };
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
    for name in ["aac.m4a", "aac-5.1.m4a", "mp2.mp4", "opus.mp4", "pcm-s24be.mov"] {
        let c = CASES.iter().find(|c| c.name == name).unwrap();
        let Some(path) = fixture(&ff, c) else { continue };
        let bytes = std::fs::read(&path).unwrap();
        let (info, whole) = decode(&bytes, None).unwrap();
        let file = isobmff::open(bytes.as_slice()).unwrap();
        let track = file.track_of_kind(isobmff::TrackKind::Audio).unwrap();
        let t = &file.tracks[track];
        let params = t.codec_params().unwrap();
        let packets: Vec<Vec<u8>> = (0..t.samples.len()).map(|i| file.read_sample(bytes.as_slice(), track, i).unwrap()).collect();
        let lookahead: Vec<&[u8]> = packets.iter().take(8).map(Vec::as_slice).collect();
        let mut dec = TrackDecoder::new(&params, &lookahead).unwrap();
        assert_eq!((dec.codec(), dec.sample_rate(), dec.layout()), (info.codec, info.sample_rate, info.layout), "{name}");
        let mut out: Vec<Vec<f32>> = vec![Vec::new(); dec.channels()];
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
        assert_eq!((list[0].codec, list[0].sample_rate, list[0].channels), (CodecId::Aac, 48_000, 1), "{ext}");
        assert_eq!((list[1].sample_rate, list[1].channels), (44_100, 2), "{ext}");
        let (first, _) = decode_with(&bytes, None, &DecodeOptions::default()).unwrap();
        let (second, buf) = decode_with(&bytes, None, &DecodeOptions::default().with_track(1)).unwrap();
        assert_eq!((first.channels, first.sample_rate), (1, 48_000), "{ext}");
        assert_eq!((second.channels, second.sample_rate, buf.frames()), (2, 44_100, 88_200), "{ext}");
        let limited = decode_with(&bytes, None, &DecodeOptions::default().with_max_samples(1000));
        assert!(matches!(limited, Err(AudioError::TooLarge(_))), "{ext}");
        assert!(decode_with(&bytes, None, &DecodeOptions::default().with_track(2)).is_err(), "{ext}: no third track");
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
