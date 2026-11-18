//! Oracle tests: compare the Ogg demuxer's streams, packet bytes and Opus timing with ffprobe on
//! ffmpeg-generated fixtures (skipped when ffmpeg/ffprobe are not installed).

mod common;

use craft_media::ogg::{self, Codec, OggFile, OpusTiming};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir() -> PathBuf {
    common::fixture_subdir("ogg")
}

/// A tone with a slow beat and a little noise, so encoders spend bits unevenly across packets.
fn music(rate: u32, layout: &str, seconds: &str) -> Vec<String> {
    let expr = format!("0.3*sin(2*PI*220*t)+0.2*sin(2*PI*331*t)*sin(2*PI*0.5*t)+0.05*random(0):s={rate}:c={layout}:d={seconds}");
    ["-f", "lavfi", "-i", &format!("aevalsrc={expr}")].map(String::from).to_vec()
}

/// Every fixture `fixture` knows (for `generate_fixtures`).
const ALL_FIXTURES: &[&str] = &[
    "opus_stereo.opus",
    "opus_mono16k_2_5ms.opus",
    "opus_60ms_odd_length.opus",
    "opus_51.opus",
    "opus_10ms.opus",
    "opus_small_pages.opus",
    "vorbis_stereo.ogg",
    "flac.oga",
    "theora_vorbis.ogv",
    "grouped.ogg",
    "chained_opus.opus",
];

/// Fixture name → ffmpeg arguments (after `-y -v error`, before the output path). `None` for a
/// fixture made from others.
fn spec(name: &str) -> Option<Vec<String>> {
    let cat = |parts: &[Vec<String>]| parts.concat();
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Some(match name {
        "opus_stereo.opus" => cat(&[music(48000, "stereo", "3"), s(&["-c:a", "libopus", "-b:a", "128k"])]),
        "opus_mono16k_2_5ms.opus" => cat(&[music(16000, "mono", "1"), s(&["-c:a", "libopus", "-frame_duration", "2.5"])]),
        "opus_60ms_odd_length.opus" => cat(&[music(48000, "stereo", "3.217"), s(&["-c:a", "libopus", "-frame_duration", "60"])]),
        "opus_51.opus" => cat(&[music(48000, "5.1", "2"), s(&["-c:a", "libopus", "-mapping_family", "1"])]),
        // one page, shorter than the pre-skip
        "opus_10ms.opus" => cat(&[music(48000, "stereo", "0.01"), s(&["-c:a", "libopus"])]),
        "opus_small_pages.opus" => cat(&[music(48000, "stereo", "2"), s(&["-c:a", "libopus", "-page_duration", "20000"])]),
        "vorbis_stereo.ogg" => cat(&[music(44100, "stereo", "3"), s(&["-c:a", "libvorbis", "-q:a", "5"])]),
        "flac.oga" => cat(&[music(44100, "stereo", "3"), s(&["-c:a", "flac", "-sample_fmt", "s16"])]),
        "theora_vorbis.ogv" => cat(&[
            s(&["-f", "lavfi", "-i", "testsrc2=s=320x240:r=25:d=2"]),
            music(48000, "stereo", "2"),
            s(&["-c:v", "libtheora", "-q:v", "6", "-c:a", "libvorbis"]),
        ]),
        // grouped (multiplexed) streams: two Opus and a Vorbis
        "grouped.ogg" => cat(&[
            music(48000, "stereo", "2"),
            music(48000, "mono", "2"),
            music(44100, "stereo", "2"),
            s(&["-map", "0", "-map", "1", "-map", "2", "-c:a:0", "libopus", "-c:a:1", "libopus", "-c:a:2", "libvorbis"]),
        ]),
        "chained_link_a.opus" => cat(&[music(48000, "stereo", "2"), s(&["-c:a", "libopus", "-fflags", "+bitexact", "-serial_offset", "100"])]),
        "chained_link_b.opus" => cat(&[music(48000, "stereo", "1.5"), s(&["-c:a", "libopus", "-fflags", "+bitexact", "-serial_offset", "200"])]),
        "chained_opus.opus" => return None,
        _ => panic!("unknown fixture {name}"),
    })
}

/// Generate (once) and return the path of a fixture; `None` after printing `SKIPPED` if generation
/// failed (a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
fn fixture(ffmpeg: &Path, name: &str) -> Option<PathBuf> {
    let dir = fixture_dir();
    let out = dir.join(name);
    if std::fs::metadata(&out).is_ok_and(|m| m.len() > 0) {
        return Some(out);
    }
    let tmp = common::temp_path(&out);
    let Some(args) = spec(name) else {
        // a chained file: one complete Ogg file after another, each with its own serial
        let mut bytes = std::fs::read(fixture(ffmpeg, "chained_link_a.opus")?).unwrap();
        bytes.extend(std::fs::read(fixture(ffmpeg, "chained_link_b.opus")?).unwrap());
        std::fs::write(&tmp, bytes).unwrap();
        std::fs::rename(&tmp, &out).unwrap();
        return Some(out);
    };
    let mut c = Command::new(ffmpeg);
    c.args(["-y", "-v", "error", "-nostdin"]).args(&args).args(["-f", "ogg"]).arg(&tmp);
    // the rename is atomic: concurrent generators race harmlessly
    common::run_generator(&mut c, &tmp, &out, common::oracles_required())
}

/// ffprobe's streams and packets, with an Adler-32 of every packet's bytes.
fn ffprobe_json(ffprobe: &Path, path: &Path) -> Value {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-show_streams", "-show_packets", "-show_data_hash", "adler32", "-of", "json"])
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert!(out.status.success(), "ffprobe failed on {}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("ffprobe json")
}

fn adler32(data: &[u8]) -> String {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    format!("adler32:{:08x}", (b << 16) | a)
}

/// One of ffprobe's packets: size, data hash, pts, duration, and the Opus trims ffmpeg sends as
/// "Skip Samples" side data (samples to skip at the start, padding to discard at the end).
#[derive(Debug)]
struct FfPacket {
    size: usize,
    hash: String,
    pts: Option<i64>,
    duration: Option<i64>,
    skip: i64,
    discard: i64,
}

fn ff_packets(json: &Value, stream: usize) -> Vec<FfPacket> {
    let packets = json["packets"].as_array().unwrap();
    packets
        .iter()
        .filter(|p| common::int(&p["stream_index"]) == Some(stream as i64))
        .map(|p| {
            let side = p["side_data_list"].as_array().and_then(|l| l.iter().find(|d| d["side_data_type"] == "Skip Samples"));
            let trim = |key: &str| side.and_then(|d| common::int(&d[key])).unwrap_or(0);
            FfPacket {
                size: common::int(&p["size"]).unwrap() as usize,
                hash: p["data_hash"].as_str().unwrap().to_string(),
                pts: common::int(&p["pts"]),
                duration: common::int(&p["duration"]),
                skip: trim("skip_samples"),
                discard: trim("discard_padding"),
            }
        })
        .collect()
}

/// The fixture, our reading of it, and ffprobe's; `None` after printing `SKIPPED`.
fn run(name: &str) -> Option<(Vec<u8>, OggFile, Value)> {
    let (ffmpeg, ffprobe) = (common::ffmpeg()?, common::ffprobe()?);
    let path = fixture(&ffmpeg, name)?;
    let bytes = std::fs::read(&path).unwrap();
    let file = ogg::open(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    let json = ffprobe_json(&ffprobe, &path);
    Some((bytes, file, json))
}

/// Our stream `s` against ffprobe's stream `ff`: codec, rate, channels, picture size, and every
/// packet byte for byte (ffprobe leaves the header packets out too). `ours` are the packets to
/// compare, so a chained file can give the links ffmpeg joins into one stream.
fn assert_stream_matches(name: &str, file: &OggFile, bytes: &[u8], ours: &[(usize, usize)], json: &Value, ff: usize) {
    let stream = &json["streams"][ff];
    let s = &file.streams[ours[0].0];
    let context = format!("{name}: stream {ff} ({})", s.codec.name());
    assert_eq!(stream["codec_name"].as_str(), Some(s.codec.name().to_lowercase().as_str()), "{context}: codec");
    let params = s.codec_params();
    if let Some(rate) = params.sample_rate {
        assert_eq!(common::int(&stream["sample_rate"]), Some(i64::from(rate)), "{context}: sample rate");
    }
    if let Some(channels) = params.channels {
        assert_eq!(common::int(&stream["channels"]), Some(i64::from(channels)), "{context}: channels");
    }
    if let (Some(w), Some(h)) = (params.width, params.height) {
        assert_eq!((common::int(&stream["width"]), common::int(&stream["height"])), (Some(i64::from(w)), Some(i64::from(h))), "{context}: size");
    }
    let theirs = ff_packets(json, ff);
    assert_eq!(ours.len(), theirs.len(), "{context}: packet count");
    for (k, (&(si, pi), p)) in ours.iter().zip(&theirs).enumerate() {
        let data = file.read_packet(bytes, si, pi).unwrap();
        assert_eq!((data.len(), adler32(&data)), (p.size, p.hash.clone()), "{context}: packet {k}");
    }
    eprintln!("{context}: {} packets byte-identical", ours.len());
}

/// Every packet of stream `s`, as (stream, packet) indices.
fn all(file: &OggFile, s: usize) -> Vec<(usize, usize)> {
    (0..file.streams[s].packets.len()).map(|i| (s, i)).collect()
}

/// `OpusTiming` against ffmpeg: the starts are its pts (both on the same clock: a stream that starts
/// at granule 0 starts at `-pre_skip`), the durations its packet durations except the last, which
/// ffmpeg trims by the discard padding, the pre-skip its first packet's skip, and the total its
/// playable length.
fn assert_opus_timing_matches(name: &str, file: &OggFile, s: usize, json: &Value, ff: usize) {
    let stream = &file.streams[s];
    let pre_skip = stream.headers.first().and_then(|h| h.get(10..12)).map_or(0, |b| u32::from(u16::from_le_bytes([b[0], b[1]])));
    let timing = OpusTiming::of(stream, pre_skip);
    let theirs = ff_packets(json, ff);
    let context = format!("{name}: Opus stream {ff}");
    assert_eq!(timing.starts.len(), theirs.len(), "{context}: packet count");
    let last = theirs.len() - 1;
    for (k, p) in theirs.iter().enumerate() {
        assert_eq!(p.pts, Some(timing.starts[k]), "{context}: start of packet {k}");
        let duration = p.duration.unwrap() + if k == last { p.discard } else { 0 };
        assert_eq!(duration, i64::from(timing.durations[k]), "{context}: duration of packet {k}");
    }
    assert_eq!(i64::from(timing.pre_skip), theirs[0].skip, "{context}: pre-skip");
    if stream.packets.first().and_then(|p| p.granule).is_some_and(|g| g >= 0) {
        assert_eq!(timing.starts[0], -i64::from(timing.pre_skip), "{context}: first start");
    }
    let playable: i64 = theirs.iter().map(|p| p.duration.unwrap()).sum::<i64>() - theirs[0].skip;
    assert_eq!(timing.total, playable, "{context}: total");
    eprintln!("{context}: timing of {} packets matches, {} samples", theirs.len(), timing.total);
}

/// A file of one Opus stream: its packets and its timing.
fn opus(name: &str) -> Option<OggFile> {
    let (bytes, file, json) = run(name)?;
    assert_eq!(file.streams.len(), 1, "{name}: {:?}", file.warnings);
    assert!(file.warnings.is_empty(), "{name}: {:?}", file.warnings);
    assert_eq!(file.streams[0].codec, Codec::Opus);
    assert_stream_matches(name, &file, &bytes, &all(&file, 0), &json, 0);
    assert_opus_timing_matches(name, &file, 0, &json, 0);
    Some(file)
}

#[test]
fn opus_stereo() {
    let Some(file) = opus("opus_stereo.opus") else { return };
    assert!(file.streams[0].saw_eos);
}

#[test]
fn opus_mono_16_khz_with_2_5_ms_frames() {
    opus("opus_mono16k_2_5ms.opus");
}

#[test]
fn opus_60_ms_frames_with_an_odd_length() {
    opus("opus_60ms_odd_length.opus");
}

#[test]
fn opus_5_1_mapping_family_1() {
    let Some(file) = opus("opus_51.opus") else { return };
    assert_eq!(file.streams[0].codec_params().channels, Some(6));
}

#[test]
fn opus_shorter_than_the_pre_skip() {
    opus("opus_10ms.opus");
}

#[test]
fn opus_with_a_page_per_packet() {
    let Some(file) = opus("opus_small_pages.opus") else { return };
    assert!(file.streams[0].pages as usize >= file.streams[0].packets.len());
}

#[test]
fn vorbis_stereo() {
    let Some((bytes, file, json)) = run("vorbis_stereo.ogg") else { return };
    assert!(file.warnings.is_empty(), "{:?}", file.warnings);
    assert_eq!(file.streams[0].headers.len(), 3);
    assert_stream_matches("vorbis_stereo.ogg", &file, &bytes, &all(&file, 0), &json, 0);
}

#[test]
fn flac_in_ogg() {
    let Some((bytes, file, json)) = run("flac.oga") else { return };
    assert!(file.warnings.is_empty(), "{:?}", file.warnings);
    assert_eq!(file.streams[0].codec, Codec::Flac);
    assert!(file.streams[0].codec_params().extradata.starts_with(b"fLaC"));
    assert_stream_matches("flac.oga", &file, &bytes, &all(&file, 0), &json, 0);
}

#[test]
fn theora_and_vorbis() {
    let Some((bytes, file, json)) = run("theora_vorbis.ogv") else { return };
    assert!(file.warnings.is_empty(), "{:?}", file.warnings);
    assert_eq!(file.streams.len(), 2);
    for s in 0..2 {
        assert_stream_matches("theora_vorbis.ogv", &file, &bytes, &all(&file, s), &json, s);
    }
    let video = file.stream_of(Codec::Theora).unwrap();
    let params = file.streams[video].codec_params();
    assert_eq!((params.width, params.height), (Some(320), Some(240)));
}

/// Multiplexed streams: each is read on its own, in ffmpeg's order.
#[test]
fn grouped_streams() {
    let Some((bytes, file, json)) = run("grouped.ogg") else { return };
    assert!(file.warnings.is_empty(), "{:?}", file.warnings);
    let codecs: Vec<Codec> = file.streams.iter().map(|s| s.codec).collect();
    assert_eq!(codecs, [Codec::Opus, Codec::Opus, Codec::Vorbis]);
    for s in 0..3 {
        assert_stream_matches("grouped.ogg", &file, &bytes, &all(&file, s), &json, s);
    }
    for s in 0..2 {
        assert_opus_timing_matches("grouped.ogg", &file, s, &json, s);
    }
}

/// A chained file: two links, each its own stream with its own headers. ffmpeg joins the links of
/// one codec into one stream.
#[test]
fn chained_opus() {
    let Some((bytes, file, json)) = run("chained_opus.opus") else { return };
    assert!(file.warnings.is_empty(), "{:?}", file.warnings);
    assert_eq!(file.streams.len(), 2);
    assert_ne!(file.streams[0].serial, file.streams[1].serial);
    assert!(file.streams.iter().all(|s| s.codec == Codec::Opus && s.headers.len() == 2 && s.saw_eos));
    assert_eq!(json["streams"].as_array().map(Vec::len), Some(1), "ffmpeg's one stream");
    let mut both = all(&file, 0);
    both.extend(all(&file, 1));
    assert_stream_matches("chained_opus.opus", &file, &bytes, &both, &json, 0);
}

/// Pre-generate every fixture (`cargo test -p craft-media --all-features -- --ignored generate_fixtures`).
#[test]
#[ignore]
fn generate_fixtures() {
    let Some(ffmpeg) = common::ffmpeg() else { return };
    for name in ALL_FIXTURES {
        let out = [fixture_dir().join(name)];
        common::generate_and_report(&format!("ogg/{name}"), &out, || fixture(&ffmpeg, name));
    }
}
