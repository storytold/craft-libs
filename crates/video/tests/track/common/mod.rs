//! Fixtures (ffmpeg encodes a short clip into a container), demuxing with craft-media, ffmpeg's
//! own decode of each fixture as the reference, and the checks every codec runs.
#![allow(dead_code)]

use craft_media::CodecParams;
use craft_video::TrackDecoder;
use craft_video::frame::{Samples, VideoFrame};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Frames in every fixture.
pub const FRAMES: usize = 24;

/// One packet of a demuxed track, in decode order.
#[derive(Clone)]
pub struct Packet {
    pub data: Vec<u8>,
    pub pts: i64,
    /// The container marks it as a random access point.
    pub key: bool,
}

/// A demuxed video track.
#[derive(Clone)]
pub struct Track {
    pub params: CodecParams,
    pub packets: Vec<Packet>,
}

/// A fixture and ffmpeg's decode of it.
pub struct Fixture {
    pub file: PathBuf,
    /// Raw frames in display order, planes one after another, 16-bit samples little-endian.
    pub reference: Vec<u8>,
    /// How far a sample may be from the reference: 0 (bit-exact) unless a test sets it.
    pub max_diff: u8,
}

/// Lowest PSNR of a frame decoded within a tolerance, as the MPEG-2 oracle tests require.
const MIN_PSNR: f64 = 58.0;

/// `name.ext` made by ffmpeg from 24 frames of a noisy test pattern at 176x144, with `args`
/// choosing the encoder, and its decode as `pix_fmt` raw video. `None` (a skip) without ffmpeg or
/// the encoder; a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`.
pub fn fixture(name: &str, ext: &str, args: &[&str], pix_fmt: &str) -> Option<Fixture> {
    let ff = craft_media_testkit::ffmpeg()?;
    let dir = craft_media_testkit::fixture_subdir("video-track");
    let required = craft_media_testkit::oracles_required();
    let file = dir.join(format!("{name}.{ext}"));
    let raw = dir.join(format!("{name}.{ext}.{pix_fmt}.raw"));
    craft_media_testkit::generate_and_report(name, &[file.clone(), raw.clone()], || {
        if !file.exists() {
            let tmp = craft_media_testkit::temp_path(&file);
            let mut cmd = Command::new(&ff);
            cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
                .arg("testsrc2=size=176x144:rate=25,noise=alls=12:allf=t+u")
                .args(["-frames:v", &FRAMES.to_string(), "-an"])
                .args(args)
                .arg(&tmp);
            craft_media_testkit::run_generator(&mut cmd, &tmp, &file, required)?;
        }
        if !raw.exists() {
            let tmp = craft_media_testkit::temp_path(&raw);
            let mut cmd = Command::new(&ff);
            cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(&file)
                .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", pix_fmt])
                .arg(&tmp);
            craft_media_testkit::run_generator(&mut cmd, &tmp, &raw, required)?;
        }
        Some(Fixture { reference: std::fs::read(&raw).unwrap(), file: file.clone(), max_diff: 0 })
    })
}

/// Whether ffmpeg has `encoder`, without skipping.
pub fn has_encoder(encoder: &str) -> bool {
    craft_media_testkit::tool("ffmpeg").is_some_and(|ff| craft_media_testkit::ffmpeg_has_encoder(&ff, encoder))
}

/// The video track of an MP4 file.
pub fn mp4(path: &Path) -> Track {
    use craft_media::isobmff::{TrackKind, open};
    let bytes = std::fs::read(path).unwrap();
    let file = open(bytes.as_slice()).unwrap();
    let t = file.track_of_kind(TrackKind::Video).unwrap();
    let track = &file.tracks[t];
    let packets = (track.samples.iter().enumerate())
        .map(|(i, s)| Packet { data: file.read_sample(bytes.as_slice(), t, i).unwrap(), pts: s.pts, key: s.is_sync })
        .collect();
    Track { params: track.codec_params(), packets }
}

/// The video stream of an MPEG transport stream.
pub fn ts(path: &Path) -> Track {
    use craft_media::mpegts::{Kind, open};
    let bytes = std::fs::read(path).unwrap();
    let file = open(bytes.as_slice()).unwrap();
    let s = file.find(Kind::Video, |_| true).unwrap();
    let stream = &file.streams[s];
    let packets = (stream.units.iter().enumerate())
        .map(|(i, u)| Packet { data: file.read_unit(bytes.as_slice(), s, i).unwrap(), pts: u.pts.unwrap(), key: u.key })
        .collect();
    Track { params: stream.codec_params(), packets }
}

/// The video stream of an AVI file. AVI has no timestamps: a chunk's index stands in.
pub fn avi(path: &Path) -> Track {
    use craft_media::avi::{StreamKind, open};
    let bytes = std::fs::read(path).unwrap();
    let file = open(bytes.as_slice()).unwrap();
    let s = file.streams.iter().position(|s| s.kind == StreamKind::Video).unwrap();
    let stream = &file.streams[s];
    let packets = (stream.chunks.iter().enumerate())
        .map(|(i, c)| Packet { data: file.read_chunk(bytes.as_slice(), s, i).unwrap(), pts: i as i64, key: c.key })
        .collect();
    Track { params: stream.codec_params(), packets }
}

/// Decode `packets` with `dec` and flush it.
pub fn run(dec: &mut TrackDecoder, packets: &[Packet]) -> Vec<VideoFrame> {
    let mut frames = Vec::new();
    for (i, p) in packets.iter().enumerate() {
        frames.extend(dec.decode(&p.data, p.pts).unwrap_or_else(|e| panic!("packet {i}: {e}")));
    }
    frames.extend(dec.flush().unwrap());
    frames
}

/// Every frame of `track`, from a new decoder.
pub fn decode(track: &Track) -> Vec<VideoFrame> {
    run(&mut TrackDecoder::new(&track.params).unwrap(), &track.packets)
}

/// The samples of `frame`, planes one after another, as ffmpeg's raw video has them: a byte per
/// sample up to 8 bits (whatever the storage; AV1 stores 8-bit samples in `u16`), else two bytes
/// little-endian.
pub fn frame_bytes(frame: &VideoFrame) -> Vec<u8> {
    let mut out = Vec::new();
    for p in frame.planes() {
        for y in 0..p.height() {
            match p.samples() {
                Samples::U8(_) => out.extend_from_slice(p.row_u8(y).unwrap()),
                Samples::U16(_) if p.bit_depth() <= 8 => out.extend(p.row_u16(y).unwrap().iter().map(|&s| u8::try_from(s).unwrap())),
                Samples::U16(_) => out.extend(p.row_u16(y).unwrap().iter().flat_map(|s| s.to_le_bytes())),
            }
        }
    }
    out
}

/// `frames` are every frame of `track` in display order, each with the timestamp of the packet
/// that coded it, and their samples are ffmpeg's (within `fixture.max_diff`).
pub fn check_frames(track: &Track, frames: &[VideoFrame], fixture: &Fixture) {
    let reference = &fixture.reference;
    assert_eq!(frames.len(), FRAMES, "frame count");
    let got: Vec<i64> = frames.iter().map(|f| f.pts.unwrap()).collect();
    let mut want: Vec<i64> = track.packets.iter().map(|p| p.pts).collect();
    want.sort_unstable();
    assert_eq!(got, want, "timestamps in display order");
    let size = reference.len() / FRAMES;
    assert_eq!(size * FRAMES, reference.len(), "reference size");
    for (i, (frame, want)) in frames.iter().zip(reference.chunks(size)).enumerate() {
        assert_eq!((frame.width(), frame.height()), (176, 144), "frame {i} size");
        let got = frame_bytes(frame);
        assert_eq!(got.len(), want.len(), "frame {i} bytes");
        let diff: Vec<usize> = (0..got.len()).filter(|&k| got[k] != want[k]).collect();
        let max = diff.iter().map(|&k| got[k].abs_diff(want[k])).max().unwrap_or(0);
        let what = format!("frame {i} (pts {:?}): {} of {} bytes differ from ffmpeg's decode, by up to {max}", frame.pts, diff.len(), got.len());
        if fixture.max_diff == 0 {
            assert!(diff.is_empty(), "{what}");
        } else {
            let sse: f64 = diff.iter().map(|&k| f64::from(got[k].abs_diff(want[k])).powi(2)).sum();
            let psnr = if sse == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64.powi(2) * got.len() as f64 / sse).log10() };
            assert!(max <= fixture.max_diff && psnr >= MIN_PSNR, "{what}, PSNR {psnr:.1} dB");
        }
    }
}

/// Decode `track`, check it against `fixture`'s reference, then reset the decoder twice: once
/// halfway through, decoding the whole track again, and once to decode from its last key packet
/// (a seek), which gives the frames of the packets from there on.
pub fn check_track(track: &Track, fixture: &Fixture) {
    let frames = decode(track);
    check_frames(track, &frames, fixture);

    let mut dec = TrackDecoder::new(&track.params).unwrap();
    let half = track.packets.len() / 2;
    for p in &track.packets[..half] {
        dec.decode(&p.data, p.pts).unwrap();
    }
    dec.reset().unwrap();
    assert!(run(&mut dec, &track.packets) == frames, "decoding again after a reset");

    let key = track.packets.iter().rposition(|p| p.key).unwrap();
    assert!(key > 0, "the track has a second key packet");
    dec.reset().unwrap();
    let seek = run(&mut dec, &track.packets[key..]);
    let pts: Vec<i64> = track.packets[key..].iter().map(|p| p.pts).collect();
    let want: Vec<&VideoFrame> = frames.iter().filter(|f| pts.contains(&f.pts.unwrap())).collect();
    assert!(seek.iter().eq(want.iter().copied()), "decoding from key packet {key} after a reset");
}
