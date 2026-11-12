//! Oracle tests for MXF (skipped without ffmpeg/ffprobe).
//!
//! Reading: ffmpeg writes OP1a, OP-Atom and D-10 files; the demuxer's tracks, sample tables, key
//! flags, timecode and PCM are compared with ffprobe's packets and ffmpeg's decoded sound.
//!
//! Writing: the essence of those files is written again with `MxfWriter` (OP1a and OP-Atom);
//! ffprobe must describe the new file as it does the source, and ffmpeg must decode the same
//! pictures and sound from it.

mod common;

use craft_media::CodecId;
use craft_media::mxf::{
    Codec, CodedKind, FrameInfo, MxfFile, MxfWriter, OpAtomPcm, OperationalPattern, PackageIds, Pattern, PictureCoding, PictureDesc, Rational,
    SoundDesc, SoundFormat, StartTimecode, TrackKind, Wrapping, WriterConfig, open, write_opatom_pcm,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const V25: &[&str] = &["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25"];
const TONE: &[&str] = &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"];
const NOISE: &[&str] = &["-f", "lavfi", "-i", "anoisesrc=color=pink:sample_rate=48000:amplitude=0.5:seed=7"];

const ALL: &[&str] = &["h264_bf2.mxf", "h264_2997df.mxf", "dnxhr_lb.mxf", "prores.mxf", "mpeg2.mxf", "atom_dnxhr.mxf", "atom_pcm.mxf", "d10.mxf"];

fn spec(name: &str) -> Vec<&'static str> {
    let cat = |p: &[&[&'static str]]| p.concat();
    let x264: &[&str] = &["-c:v", "libx264", "-pix_fmt", "yuv420p"];
    match name {
        // long-GOP H.264 with B pictures (ffmpeg writes no temporal offsets)
        "h264_bf2.mxf" => {
            cat(&[V25, NOISE, &["-t", "2"], x264, &["-bf", "2", "-g", "12", "-c:a", "pcm_s16le", "-ac", "2", "-timecode", "01:00:00:00"]])
        }
        "h264_2997df.mxf" => cat(&[
            &["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30000/1001"],
            NOISE,
            &["-t", "1"],
            x264,
            &["-bf", "0", "-g", "10", "-c:a", "pcm_s24le", "-ac", "2", "-timecode", "01:00:00;00"],
        ]),
        "dnxhr_lb.mxf" => {
            cat(&[V25, TONE, &["-t", "1", "-c:v", "dnxhd", "-profile:v", "dnxhr_lb", "-pix_fmt", "yuv422p", "-c:a", "pcm_s24le", "-ac", "2"]])
        }
        "prores.mxf" => cat(&[V25, TONE, &["-t", "1", "-c:v", "prores_ks", "-profile:v", "3", "-pix_fmt", "yuv422p10le", "-c:a", "pcm_s16le"]]),
        "mpeg2.mxf" => cat(&[V25, NOISE, &["-t", "1", "-c:v", "mpeg2video", "-bf", "2", "-c:a", "pcm_s16le"]]),
        "atom_dnxhr.mxf" => cat(&[V25, &["-t", "1", "-c:v", "dnxhd", "-profile:v", "dnxhr_lb", "-pix_fmt", "yuv422p", "-f", "mxf_opatom"]]),
        "atom_pcm.mxf" => cat(&[NOISE, &["-t", "1", "-c:a", "pcm_s24le", "-ac", "1", "-f", "mxf_opatom"]]),
        // D-10 (IMX): intra MPEG-2 at a constant rate, sound as 8-channel AES3 elements
        "d10.mxf" => cat(&[
            &["-f", "lavfi", "-i", "testsrc2=size=720x608:rate=25"],
            NOISE,
            &["-t", "0.4", "-c:v", "mpeg2video", "-pix_fmt", "yuv422p", "-minrate", "30M", "-maxrate", "30M", "-b:v", "30M"],
            &["-bufsize", "1200000", "-rc_init_occupancy", "1200000", "-intra_vlc", "1", "-non_linear_quant", "1", "-g", "1"],
            &["-flags", "+ildct+low_delay", "-dc", "10", "-ps", "1", "-qmin", "1", "-qmax", "3", "-c:a", "pcm_s16le", "-ac", "4", "-f", "mxf_d10"],
        ]),
        other => panic!("unknown fixture {other}"),
    }
}

fn fixture(name: &str) -> Option<PathBuf> {
    let ffmpeg = common::ffmpeg_or_skip("mxf oracle")?;
    let out = common::fixtures_dir("mxf").join(name);
    common::fixtures::generate(&out, |tmp: &Path| {
        let mut c = Command::new(&ffmpeg);
        c.args(["-v", "error", "-y", "-nostdin"]).args(spec(name));
        // OP1a is chosen by the `.mxf` extension; the others name their muxer
        c.arg(tmp).status().is_ok_and(|s| s.success())
    })
}

/// ffprobe's packets of stream `index`: (pts, size, key).
fn packets(all: &[Value], index: i64) -> Vec<(Option<i64>, u64, bool)> {
    all.iter()
        .filter(|p| common::int(&p["stream_index"]) == Some(index))
        .map(|p| (common::int(&p["pts"]), common::int(&p["size"]).unwrap_or(0) as u64, p["flags"].as_str().is_some_and(|f| f.starts_with('K'))))
        .collect()
}

/// ffmpeg's decode of the first sound stream as planar f32 (`channels` channels).
fn ffmpeg_pcm(path: &Path, channels: usize) -> Option<Vec<Vec<f32>>> {
    let ff = common::ffmpeg()?;
    let out = Command::new(ff)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
    let samples: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
    Some((0..channels).map(|c| samples.iter().skip(c).step_by(channels).copied().collect()).collect())
}

/// MD5 of ffmpeg's decode of one stream (`v:0` or `a:0`), and whatever it complained about.
fn decode_md5(path: &Path, stream: &str) -> Option<String> {
    let ff = common::ffmpeg()?;
    let out = Command::new(ff)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", &format!("0:{stream}"), "-f", "md5", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn expected_codec(name: &str) -> Codec {
    match name {
        "h264" => Codec::Avc { intra: false },
        "dnxhd" => Codec::Vc3,
        "prores" => Codec::ProRes { profile: None },
        "mpeg2video" => Codec::Mpeg2,
        other => panic!("ffprobe codec {other} has no mapping in this test"),
    }
}

struct Opened {
    path: PathBuf,
    bytes: Vec<u8>,
    mxf: MxfFile,
}

/// Open a fixture and compare every track with ffprobe.
fn check(name: &str) -> Option<Opened> {
    let path = fixture(name)?;
    let bytes = std::fs::read(&path).unwrap();
    let mxf = open(&bytes).unwrap();
    // ffmpeg's D-10 files have no index table: the key frames come from a scan, with a warning
    let expected: &[&str] =
        if name == "d10.mxf" { &["no index table with random-access flags: key frames found by scanning the essence"] } else { &[] };
    assert_eq!(mxf.warnings, expected, "{name}");
    let streams = common::streams(&path)?;
    let all = common::packets(&path)?;
    let av: Vec<&Value> = streams.iter().filter(|s| s["codec_type"] == "video" || s["codec_type"] == "audio").collect();
    // a D-10 file's four channels are one sound track here and one stream in ffprobe
    assert_eq!(mxf.tracks.len(), av.len(), "{name}: track count");
    for (t, s) in mxf.tracks.iter().zip(&av) {
        let index = common::int(&s["index"]).unwrap();
        let pk = packets(&all, index);
        let params = t.codec_params();
        if s["codec_type"] == "video" {
            assert_eq!(t.kind, TrackKind::Picture, "{name}");
            let want = expected_codec(s["codec_name"].as_str().unwrap());
            // the ProRes profile comes from the coding label
            assert_eq!(std::mem::discriminant(&t.codec), std::mem::discriminant(&want), "{name}: {:?}", t.codec);
            let (w, h) = (common::int(&s["width"]).unwrap() as u32, common::int(&s["height"]).unwrap() as u32);
            if name == "d10.mxf" {
                // D-10 stores 608 lines (two fields of 304, with 32 lines above the picture) and
                // displays 576: ffprobe reports the stored size, `codec_params` the displayed one
                let pic = t.picture.as_ref().unwrap();
                assert_eq!((pic.stored_width, pic.stored_height * 2), (w, h), "{name}");
                assert_eq!((params.width, params.height), (Some(720), Some(576)), "{name}");
            } else {
                assert_eq!((params.width, params.height), (Some(w), Some(h)), "{name}");
            }
            assert_eq!(t.samples.len(), pk.len(), "{name}: picture count");
            for (i, (sample, p)) in t.samples.iter().zip(&pk).enumerate() {
                assert_eq!(u64::from(sample.size), p.1, "{name}: picture {i} size");
                if t.codec == Codec::Mpeg2 {
                    // ffprobe marks every I picture; `key` is the index's random-access bit, which
                    // the I picture of an open GOP does not have (see `op1a_files_match_ffprobe`)
                    assert!(!sample.key || p.2, "{name}: picture {i} is a key only here");
                } else {
                    assert_eq!(sample.key, p.2, "{name}: picture {i} key flag");
                }
            }
            assert!(t.samples.first().is_some_and(|s| s.key), "{name}: the first picture is a key");
            // presentation order, where the index has temporal offsets: ffprobe's PTS order
            if t.temporal_offsets {
                let first = pk.iter().filter_map(|p| p.0).min().unwrap();
                let theirs: Vec<i64> = pk.iter().map(|p| p.0.unwrap() - first).collect();
                let ours: Vec<i64> = t.samples.iter().map(|s| s.pts).collect();
                assert_eq!(ours, theirs, "{name}: presentation positions");
            }
            let mut order = t.display_order.clone();
            order.sort_unstable();
            assert_eq!(order, (0..t.samples.len()).collect::<Vec<_>>(), "{name}: the display order is a permutation");
        } else {
            assert_eq!(t.kind, TrackKind::Sound, "{name}");
            let info = t.sound.as_ref().unwrap();
            let channels = common::int(&s["channels"]).unwrap() as usize;
            assert_eq!(info.channels as usize, channels, "{name}: channels");
            assert_eq!((params.codec, params.sample_rate), (CodecId::Pcm, Some(48_000)), "{name}");
            // every stored sample, exactly as ffmpeg decodes it
            let theirs = ffmpeg_pcm(&path, channels)?;
            let frames = theirs[0].len();
            assert_eq!(t.stored_sample_frames(), frames as u64, "{name}: sample frames");
            let track = mxf.tracks.iter().position(|x| std::ptr::eq(x, t)).unwrap();
            let ours = mxf.read_pcm(&bytes, track, 0, frames).unwrap();
            assert!(ours == theirs, "{name}: PCM differs from ffmpeg's decode");
            // and a read from the middle
            let part = mxf.read_pcm(&bytes, track, 1000, 5000).unwrap();
            assert!(part.iter().zip(&theirs).all(|(a, b)| a[..] == b[1000..6000]), "{name}: PCM from sample 1000");
        }
    }
    Some(Opened { path, bytes, mxf })
}

#[test]
fn op1a_files_match_ffprobe() {
    for name in ["h264_bf2.mxf", "h264_2997df.mxf", "dnxhr_lb.mxf", "prores.mxf", "mpeg2.mxf"] {
        let Some(o) = check(name) else { return };
        let m = &o.mxf;
        assert_eq!(m.operational_pattern, OperationalPattern::Generalized { item: 1, package: 1 }, "{name}");
        assert!(m.tracks.iter().all(|t| t.wrapping == Wrapping::Frame), "{name}");
        let v = &m.tracks[m.track_of_kind(TrackKind::Picture).unwrap()];
        match name {
            "h264_bf2.mxf" => {
                assert_eq!(m.timecode.map(|t| t.format()).as_deref(), Some("01:00:00:00"));
                // B pictures in the index, no temporal offsets: the order is the caller's to find
                assert!(v.needs_reorder && !v.temporal_offsets && v.samples.iter().any(|s| s.b_picture));
                assert!(v.samples.iter().filter(|s| s.key).count() > 1);
            }
            "h264_2997df.mxf" => {
                assert_eq!(m.timecode.map(|t| t.format()).as_deref(), Some("01:00:00;00"));
                assert_eq!(v.edit_rate, Rational::new(30_000, 1001));
                assert!(!v.needs_reorder);
            }
            // ffmpeg's profile 3 is 422 HQ, 4 in the RDD 44 coding label
            "prores.mxf" => assert_eq!(v.codec, Codec::ProRes { profile: Some(4) }),
            "dnxhr_lb.mxf" => assert!(v.samples.iter().all(|s| s.key) && v.codec.intra_only()),
            _ => {
                // long-GOP MPEG-2 with open GOPs: the first I picture is a random-access point,
                // the later ones are I pictures (index flags 00xx) without the random-access bit
                let flags: Vec<u8> = m.index_segments.iter().flat_map(|s| s.entries.iter().map(|e| e.flags)).collect();
                assert_eq!(flags.len(), v.samples.len());
                assert!(flags[0] & 0x80 != 0 && v.samples[0].key);
                let open_gop: Vec<usize> = (1..flags.len()).filter(|&i| flags[i] & 0x30 == 0 && flags[i] & 0x80 == 0).collect();
                assert!(!open_gop.is_empty(), "flags {flags:02x?}");
                assert!(open_gop.iter().all(|&i| !v.samples[i].key));
            }
        }
    }
}

#[test]
fn op_atom_and_d10_files_match_ffprobe() {
    let Some(o) = check("atom_dnxhr.mxf") else { return };
    assert_eq!(o.mxf.operational_pattern, OperationalPattern::Atom);
    assert_eq!((o.mxf.tracks[0].wrapping, o.mxf.tracks[0].codec), (Wrapping::Clip, Codec::Vc3));
    let Some(o) = check("atom_pcm.mxf") else { return };
    assert_eq!(o.mxf.operational_pattern, OperationalPattern::Atom);
    assert_eq!(o.mxf.tracks[0].sound_format, SoundFormat::Pcm);
    assert!(o.mxf.tracks[0].codec_params().pcm.is_some_and(|p| p.bits == 24 && !p.big_endian));
    let Some(o) = check("d10.mxf") else { return };
    let a = &o.mxf.tracks[o.mxf.track_of_kind(TrackKind::Sound).unwrap()];
    assert_eq!((a.sound_format, a.codec), (SoundFormat::Aes3Element, Codec::Aes3Element));
    assert_eq!(a.codec_params().pcm, None, "AES3 element data is not plain PCM");
    assert_eq!(o.mxf.tracks[o.mxf.track_of_kind(TrackKind::Picture).unwrap()].codec, Codec::Mpeg2);
}

/// `profile_idc` of the first SPS in an Annex B access unit.
fn avc_profile(au: &[u8]) -> Option<u8> {
    au.windows(5).find(|w| w[..3] == [0, 0, 1] && w[3] & 0x1F == 7).map(|w| w[4])
}

/// The writer's picture coding for a track read from a file.
fn coding_of(codec: Codec, first: &[u8]) -> PictureCoding {
    match codec {
        // the VC-3 compression id is at byte 40 of the frame header
        Codec::Vc3 => PictureCoding::Vc3 { cid: u32::from_be_bytes(first[40..44].try_into().unwrap()) },
        Codec::ProRes { profile } => PictureCoding::ProRes { profile: profile.unwrap() },
        Codec::Avc { intra } => PictureCoding::Avc { profile_idc: avc_profile(first).unwrap(), intra },
        other => panic!("the writer has no mapping for {other:?}"),
    }
}

/// Write the essence of an OP1a file again as OP1a: every picture and the raw sound bytes.
fn rewrite_op1a(o: &Opened, name: &str) -> PathBuf {
    let m = &o.mxf;
    let vi = m.track_of_kind(TrackKind::Picture).unwrap();
    let ai = m.track_of_kind(TrackKind::Sound).unwrap();
    let (v, a) = (&m.tracks[vi], &m.tracks[ai]);
    let first = m.read_sample(&o.bytes, vi, 0).unwrap();
    let pic = v.picture.as_ref().unwrap();
    let mut cfg = WriterConfig::new(Pattern::Op1a, v.edit_rate, PackageIds::from_seed(name, "Rewritten"));
    cfg.picture = Some(PictureDesc::new(coding_of(v.codec, &first), pic.frame_width(), pic.frame_height()));
    let s = a.sound.as_ref().unwrap();
    cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: s.channels, bits: s.bits }];
    let tc = m.timecode.unwrap();
    cfg.timecode = Some(StartTimecode { frames: tc.start, rate: v.edit_rate, drop_frame: tc.drop_frame });
    let out = common::fixtures_dir("mxf").join(format!("rewritten_{name}"));
    let mut w = MxfWriter::new(std::io::BufWriter::new(std::fs::File::create(&out).unwrap()), cfg).unwrap();
    for (i, sample) in v.samples.iter().enumerate() {
        let data = m.read_sample(&o.bytes, vi, i).unwrap();
        // no B pictures in these files: stored order is presentation order
        let info = if sample.key { FrameInfo::intra() } else { FrameInfo { key: false, kind: CodedKind::Predicted, display: None } };
        w.push_picture(&data, info).unwrap();
    }
    for c in &a.chunks {
        w.push_sound(0, &o.bytes[c.offset as usize..(c.offset + c.size) as usize]).unwrap();
    }
    assert_eq!(w.edit_units(), v.samples.len() as u64);
    w.finish().unwrap();
    out
}

/// Files written by `MxfWriter` from ffmpeg-encoded essence: ffprobe describes them as it does
/// the source, ffmpeg decodes the same pictures and sound, and our own reader reads them back.
#[test]
fn written_op1a_files_are_read_by_ffmpeg_like_their_source() {
    for name in ["dnxhr_lb.mxf", "prores.mxf", "h264_2997df.mxf"] {
        let Some(o) = check(name) else { return };
        let out = rewrite_op1a(&o, name);
        let (Some(src), Some(new)) = (common::streams(&o.path), common::streams(&out)) else { return };
        for (a, b) in src.iter().zip(&new) {
            for key in ["codec_type", "codec_name", "width", "height", "pix_fmt", "sample_rate", "channels", "r_frame_rate", "bits_per_sample"] {
                assert_eq!(a[key], b[key], "{name}: stream {key}");
            }
        }
        assert_eq!(src.len(), new.len(), "{name}: stream count");
        let (Some(pa), Some(pb)) = (common::packets(&o.path), common::packets(&out)) else { return };
        for index in 0..2 {
            let (a, b) = (packets(&pa, index), packets(&pb, index));
            assert_eq!(a.len(), b.len(), "{name}: stream {index} packet count");
            if index == 0 {
                assert!(a.iter().zip(&b).all(|(x, y)| (x.1, x.2) == (y.1, y.2)), "{name}: picture sizes and key flags");
            } else {
                // at 29.97 an edit unit holds 1601 or 1602 samples, in an order each writer picks
                assert_eq!(a.iter().map(|p| p.1).sum::<u64>(), b.iter().map(|p| p.1).sum::<u64>(), "{name}: sound bytes");
            }
        }
        let (Some(fa), Some(fb)) = (common::probe_json(&o.path, &["-show_format"]), common::probe_json(&out, &["-show_format"])) else { return };
        assert_eq!(fb["format"]["format_name"], "mxf", "{name}");
        assert_eq!(fa["format"]["tags"]["timecode"], fb["format"]["tags"]["timecode"], "{name}: timecode");
        assert_eq!(fb["format"]["tags"]["product_name"], "craft-media", "{name}");
        for stream in ["v:0", "a:0"] {
            assert_eq!(decode_md5(&o.path, stream), decode_md5(&out, stream), "{name}: ffmpeg's decode of {stream}");
        }
        // and our reader gives back what went in
        let bytes = std::fs::read(&out).unwrap();
        let m = open(&bytes).unwrap();
        assert!(m.warnings.is_empty(), "{name}: {:?}", m.warnings);
        assert_eq!(m.operational_pattern, OperationalPattern::Generalized { item: 1, package: 1 });
        let (vi, src_vi) = (m.track_of_kind(TrackKind::Picture).unwrap(), o.mxf.track_of_kind(TrackKind::Picture).unwrap());
        for i in 0..m.tracks[vi].samples.len() {
            assert_eq!(m.read_sample(&bytes, vi, i).unwrap(), o.mxf.read_sample(&o.bytes, src_vi, i).unwrap(), "{name}: picture {i}");
        }
    }
}

#[test]
fn written_op_atom_files_are_read_by_ffmpeg() {
    // picture: the DNxHR frames of ffmpeg's OP-Atom file, clip-wrapped again
    let Some(o) = check("atom_dnxhr.mxf") else { return };
    let v = &o.mxf.tracks[0];
    let first = o.mxf.read_sample(&o.bytes, 0, 0).unwrap();
    let mut cfg = WriterConfig::new(Pattern::OpAtom, v.edit_rate, PackageIds::from_seed("atom", "Atom"));
    cfg.picture = Some(PictureDesc::new(coding_of(v.codec, &first), 320, 240));
    let mut w = MxfWriter::new(std::io::Cursor::new(Vec::new()), cfg).unwrap();
    for i in 0..v.samples.len() {
        w.push_picture(&o.mxf.read_sample(&o.bytes, 0, i).unwrap(), FrameInfo::intra()).unwrap();
    }
    let out = common::fixtures_dir("mxf").join("rewritten_atom_dnxhr.mxf");
    std::fs::write(&out, w.finish().unwrap().into_inner()).unwrap();
    let Some(streams) = common::streams(&out) else { return };
    assert_eq!((streams.len(), &streams[0]["codec_name"]), (1, &Value::from("dnxhd")));
    let Some(all) = common::packets(&out) else { return };
    assert_eq!(all.len(), v.samples.len());
    assert_eq!(decode_md5(&o.path, "v:0"), decode_md5(&out, "v:0"));

    // sound: 24-bit samples, read by ffmpeg bit for bit
    let samples: Vec<i32> = (0..48_000).map(|k| ((k * 7919) % 16_777_216) - 8_388_608).collect();
    for (label, edit_rate) in [("samples", None), ("frames", Some(Rational::new(25, 1)))] {
        let opts = OpAtomPcm { sample_rate: 48_000, bits: 24, channels: 1, edit_rate, ids: PackageIds::from_seed(label, "A1"), timecode: None };
        let out = common::fixtures_dir("mxf").join(format!("written_atom_pcm_{label}.mxf"));
        std::fs::write(&out, write_opatom_pcm(&opts, &samples).unwrap()).unwrap();
        let Some(pcm) = ffmpeg_pcm(&out, 1) else { return };
        let want: Vec<f32> = samples.iter().map(|s| *s as f32 / 8_388_608.0).collect();
        assert!(pcm[0] == want, "{label}: ffmpeg decodes other samples");
        let Some(streams) = common::streams(&out) else { return };
        assert_eq!((&streams[0]["codec_name"], &streams[0]["sample_rate"]), (&Value::from("pcm_s24le"), &Value::from("48000")));
    }
}

/// Every fixture, truncated and damaged: opening and reading never panics.
#[test]
fn damaged_ffmpeg_files_never_panic() {
    for name in ALL {
        let Some(path) = fixture(name) else { return };
        let bytes = std::fs::read(&path).unwrap();
        let mut x = 0x2545_F491u32;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as usize
        };
        // damage where the structure is: the header and the footer, not the essence in between
        let edges = 16_384.min(bytes.len() / 2);
        for round in 0..60 {
            let mut d = bytes.clone();
            if round % 4 == 0 {
                d.truncate(next() % d.len());
            }
            for _ in 0..1 + round % 12 {
                let at = if next() % 2 == 0 { next() % edges } else { d.len().saturating_sub(1 + next() % edges) };
                if let Some(b) = d.get_mut(at) {
                    *b = [0, 0xFF, 0x80, next() as u8][next() % 4];
                }
            }
            let Ok(m) = open(&d) else { continue };
            for (ti, t) in m.tracks.iter().enumerate() {
                let _ = (t.codec_params(), t.sync_before(t.samples.len()), m.timecode.map(|t| t.format()));
                let n = t.samples.len();
                for i in (0..n).step_by((n / 8).max(1)).chain(n.checked_sub(1)) {
                    let _ = m.read_sample(&d, ti, i);
                }
                let _ = m.read_pcm(&d, ti, 0, 4800);
                let _ = m.read_pcm(&d, ti, t.stored_sample_frames().saturating_sub(100), 4800);
            }
        }
    }
}
