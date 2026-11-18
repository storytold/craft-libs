//! Oracle tests: ffmpeg-made transport and program streams (skipped without ffmpeg/ffprobe). The
//! demuxer's streams, access-unit counts, sizes, timestamps and key flags are compared with
//! ffprobe's packets, and every unit reads back.

mod common;

use craft_media::CodecId;
use craft_media::mpegts::{Codec, File, Format, Kind, open};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const PIC: &[&str] = &["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25"];
/// Stereo audio with different content per channel.
const NOISE: &[&str] =
    &["-f", "lavfi", "-i", "aevalsrc=exprs=0.3*sin(2*PI*441*t)+0.1*sin(2*PI*3100*t)|0.25*sin(2*PI*660*t)+0.1*sin(2*PI*(500+4000*t)*t):s=48000"];

const ALL: &[&str] = &[
    "ts_mpeg2_mp2.ts",
    "ts_h264_aac.ts",
    "m2ts_h264_lpcm.m2ts",
    "m2ts_h264_ac3.mts",
    "ts_hevc_latm.ts",
    "ts_h264_eac3.ts",
    "ps_mpeg2_mp2.vob",
    "ps_mpeg2_lpcm.vob",
    "ps_mpeg2_ac3.vob",
    "mpeg1_system.mpg",
];

fn spec(name: &str) -> Vec<&'static str> {
    let cat = |p: &[&[&'static str]]| p.concat();
    let x264: &[&str] = &["-c:v", "libx264", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p"];
    match name {
        // broadcast-style: interlaced MPEG-2 and MPEG-1 layer II
        "ts_mpeg2_mp2.ts" => cat(&[
            &["-f", "lavfi", "-i", "testsrc2=size=720x576:rate=50,tinterlace=mode=interleave_top,setfield=tff"],
            NOISE,
            &[
                "-t",
                "1.2",
                "-c:v",
                "mpeg2video",
                "-flags",
                "+ilme+ildct",
                "-bf",
                "2",
                "-g",
                "12",
                "-b:v",
                "6M",
                "-c:a",
                "mp2",
                "-b:a",
                "192k",
                "-f",
                "mpegts",
            ],
        ]),
        "ts_h264_aac.ts" => cat(&[PIC, NOISE, &["-t", "1.2"], x264, &["-c:a", "aac", "-b:a", "128k", "-f", "mpegts"]]),
        // BDAV (192-byte packets) with Blu-ray LPCM, and with AC-3 as AVCHD has
        "m2ts_h264_lpcm.m2ts" => {
            cat(&[PIC, NOISE, &["-t", "1"], x264, &["-c:a", "pcm_bluray", "-ac", "2", "-f", "mpegts", "-mpegts_m2ts_mode", "1"]])
        }
        "m2ts_h264_ac3.mts" => {
            cat(&[PIC, NOISE, &["-t", "1"], x264, &["-c:a", "ac3", "-b:a", "192k", "-ac", "2", "-f", "mpegts", "-mpegts_m2ts_mode", "1"]])
        }
        "ts_hevc_latm.ts" => cat(&[
            PIC,
            NOISE,
            &["-t", "1", "-c:v", "libx265", "-x265-params", "log-level=error:bframes=2:keyint=12"],
            &["-c:a", "aac", "-b:a", "96k", "-f", "mpegts", "-mpegts_flags", "latm"],
        ]),
        "ts_h264_eac3.ts" => cat(&[PIC, NOISE, &["-t", "1"], x264, &["-c:a", "eac3", "-b:a", "192k", "-f", "mpegts"]]),
        // DVD-style program streams
        "ps_mpeg2_mp2.vob" => cat(&[
            &["-f", "lavfi", "-i", "testsrc2=size=720x480:rate=30000/1001"],
            NOISE,
            &["-t", "1.2", "-c:v", "mpeg2video", "-bf", "2", "-g", "15", "-b:v", "5M", "-c:a", "mp2", "-b:a", "224k", "-f", "vob"],
        ]),
        "ps_mpeg2_lpcm.vob" => cat(&[PIC, NOISE, &["-t", "1", "-c:v", "mpeg2video", "-bf", "2", "-c:a", "pcm_dvd", "-ac", "2", "-f", "vob"]]),
        "ps_mpeg2_ac3.vob" => cat(&[PIC, NOISE, &["-t", "1", "-c:v", "mpeg2video", "-bf", "2", "-c:a", "ac3", "-b:a", "192k", "-f", "vob"]]),
        // ISO/IEC 11172-1 system stream
        "mpeg1_system.mpg" => {
            cat(&[PIC, NOISE, &["-t", "1.2", "-c:v", "mpeg1video", "-bf", "2", "-b:v", "1500k", "-c:a", "mp2", "-b:a", "128k", "-f", "mpeg"]])
        }
        other => panic!("unknown fixture {other}"),
    }
}

fn fixture(name: &str) -> Option<PathBuf> {
    let ffmpeg = common::ffmpeg_or_skip("mpegts oracle")?;
    let out = common::fixtures_dir("mpegts").join(name);
    common::fixtures::generate(&out, |tmp: &Path| {
        // the temporary name keeps no known extension: every spec names its muxer with `-f`
        Command::new(&ffmpeg).args(["-v", "error", "-y", "-nostdin"]).args(spec(name)).arg(tmp).status().is_ok_and(|s| s.success())
    })
}

/// ffprobe's packets of stream `index`: (pts, size, key).
fn packets(all: &[Value], index: i64) -> Vec<(Option<i64>, u64, bool)> {
    all.iter()
        .filter(|p| common::int(&p["stream_index"]) == Some(index))
        .map(|p| (common::int(&p["pts"]), common::int(&p["size"]).unwrap_or(0) as u64, p["flags"].as_str().is_some_and(|f| f.starts_with('K'))))
        .collect()
}

/// The codec ffprobe names, as this module's `Codec`.
fn expected_codec(name: &str, format: Format) -> Codec {
    match name {
        "mpeg1video" => Codec::Mpeg1Video,
        "mpeg2video" => Codec::Mpeg2Video,
        "h264" => Codec::H264,
        "hevc" => Codec::Hevc,
        "mp2" | "mp3" => Codec::MpegAudio,
        "aac" => Codec::AacAdts,
        "aac_latm" => Codec::AacLatm,
        "ac3" => Codec::Ac3,
        "eac3" => Codec::Eac3,
        "pcm_bluray" => Codec::LpcmBluray,
        "pcm_dvd" => Codec::LpcmDvd,
        other => panic!("{format:?}: ffprobe codec {other} has no mapping in this test"),
    }
}

fn check(name: &str) -> Option<File> {
    let path = fixture(name)?;
    let bytes = std::fs::read(&path).unwrap();
    let f = open(&bytes).unwrap();
    assert!(f.warnings.is_empty(), "{name}: {:?}", f.warnings);
    let streams = common::streams(&path)?;
    let all = common::packets(&path)?;
    assert_eq!(f.streams.len(), streams.len(), "{name}: stream count");
    for kind in [Kind::Video, Kind::Audio] {
        let what = if kind == Kind::Video { "video" } else { "audio" };
        // a program stream lists its streams in the order they appear, which need not be ffprobe's
        let ours = f.streams.iter().position(|s| s.kind() == kind).unwrap_or_else(|| panic!("{name}: no {what} stream"));
        let theirs = streams.iter().find(|s| s["codec_type"] == what).unwrap_or_else(|| panic!("{name}: ffprobe has no {what} stream"));
        let s = &f.streams[ours];
        let codec = expected_codec(theirs["codec_name"].as_str().unwrap(), f.format);
        assert_eq!(s.codec, codec, "{name}: {what} codec");
        let pk = packets(&all, common::int(&theirs["index"]).unwrap());
        assert_eq!(s.units.len(), pk.len(), "{name}: {what} unit count");
        if kind == Kind::Video {
            // every picture ffprobe gives a PTS has the same one here (ffprobe fills in some a
            // program stream leaves out), in the same order
            for (i, (u, p)) in s.units.iter().zip(&pk).enumerate() {
                if let Some(pts) = u.pts {
                    assert_eq!(Some(pts), p.0, "{name}: video unit {i} PTS");
                }
                assert_eq!(u.key, p.2, "{name}: video unit {i} key flag");
            }
            let stamped = s.units.iter().filter(|u| u.pts.is_some()).count();
            assert!(stamped > 0, "{name}: no video unit has a PTS");
            if matches!(f.format, Format::Ts { .. }) {
                assert_eq!(stamped, s.units.len(), "{name}: every TS picture starts a PES packet");
            }
        } else {
            // LATM: ffprobe's packets are the AAC frames without the LOAS framing
            if codec != Codec::AacLatm {
                let (ours, theirs): (Vec<u64>, Vec<u64>) = (s.units.iter().map(|u| u64::from(u.size)).collect(), pk.iter().map(|p| p.1).collect());
                // a DVD LPCM unit is the samples alone; ffprobe's packet keeps the 3 format bytes
                // before them
                let kept = if codec == Codec::LpcmDvd { 3 } else { 0 };
                assert_eq!(ours.iter().map(|n| n + kept).collect::<Vec<_>>(), theirs, "{name}: audio unit sizes");
            }
            // a PTS belongs to the first frame that starts in a PES packet; ffprobe interpolates
            // the others
            for (i, (u, p)) in s.units.iter().zip(&pk).enumerate() {
                if let Some(pts) = u.pts {
                    assert_eq!(Some(pts), p.0, "{name}: audio unit {i} PTS");
                }
            }
            assert!(s.units.first().is_some_and(|u| u.pts.is_some()), "{name}: the first audio unit has a PTS");
            assert!(s.units.iter().all(|u| u.key), "{name}: audio units are all key");
        }
        // every unit reads back whole
        for (i, u) in s.units.iter().enumerate() {
            assert_eq!(f.read_unit(&bytes, ours, i).unwrap().len(), u.size as usize, "{name}: {what} unit {i}");
        }
    }
    Some(f)
}

#[test]
fn transport_streams_match_ffprobe() {
    for name in ["ts_mpeg2_mp2.ts", "ts_h264_aac.ts", "ts_hevc_latm.ts", "ts_h264_eac3.ts"] {
        let Some(f) = check(name) else { return };
        assert_eq!(f.format, Format::Ts { packet_size: 188, sync_offset: 0 }, "{name}");
        assert_eq!(f.program, Some(1), "{name}");
        let (first, last) = f.pcr_range.unwrap();
        assert!(last > first, "{name}: PCR range");
        let ids: Vec<CodecId> = f.streams.iter().map(|s| s.codec_params().codec).collect();
        let want = match name {
            "ts_mpeg2_mp2.ts" => [CodecId::Mpeg2Video, CodecId::Mp2],
            "ts_h264_aac.ts" => [CodecId::H264, CodecId::Aac],
            "ts_hevc_latm.ts" => [CodecId::Hevc, CodecId::Aac],
            _ => [CodecId::H264, CodecId::Eac3],
        };
        assert_eq!(ids, want, "{name}");
    }
}

#[test]
fn mpeg2_pictures_carry_their_coding_type_and_field_order() {
    let Some(f) = check("ts_mpeg2_mp2.ts") else { return };
    let v = &f.streams[0];
    let pics: Vec<_> = v.units.iter().map(|u| u.picture.unwrap()).collect();
    assert_eq!(pics[0].coding_type, 1);
    assert!(pics[0].sequence_header && pics[0].gop);
    assert!(pics.iter().any(|p| p.coding_type == 3) && pics.iter().any(|p| p.coding_type == 2));
    assert!(pics.iter().all(|p| p.top_field_first && !p.progressive_frame), "interlaced, top field first");
    // B pictures are the disposable ones, I pictures the keys
    for (u, p) in v.units.iter().zip(&pics) {
        assert_eq!((u.key, u.disposable), (p.coding_type == 1, p.coding_type == 3));
    }
}

#[test]
fn bdav_streams_match_ffprobe() {
    for name in ["m2ts_h264_lpcm.m2ts", "m2ts_h264_ac3.mts"] {
        let Some(f) = check(name) else { return };
        assert_eq!(f.format, Format::Ts { packet_size: 192, sync_offset: 4 }, "{name}");
        assert_eq!(f.format.name(), "MPEG-2 TS (BDAV/AVCHD)");
    }
}

#[test]
fn program_streams_match_ffprobe() {
    for name in ["ps_mpeg2_mp2.vob", "ps_mpeg2_lpcm.vob", "ps_mpeg2_ac3.vob", "mpeg1_system.mpg"] {
        let Some(f) = check(name) else { return };
        assert_eq!(f.format, Format::Ps { mpeg1: name == "mpeg1_system.mpg" }, "{name}");
        assert!(f.pcr_range.is_some(), "{name}: SCR range");
        if name == "ps_mpeg2_lpcm.vob" {
            let a = f.streams.iter().find(|s| s.kind() == Kind::Audio).unwrap();
            let p = a.codec_params();
            assert_eq!((p.codec, p.sample_rate, p.channels), (CodecId::Pcm, Some(48_000), Some(2)));
            assert!(p.pcm.is_some_and(|f| f.big_endian && f.signed && !f.float));
        }
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
        for round in 0..12 {
            let mut d = bytes.clone();
            if round % 3 == 0 {
                d.truncate(next() % d.len());
            }
            for _ in 0..(round * 40) {
                let at = next() % d.len().max(1);
                if let Some(b) = d.get_mut(at) {
                    *b = next() as u8;
                }
            }
            let Ok(f) = open(&d) else { continue };
            for (s, st) in f.streams.iter().enumerate() {
                let _ = st.codec_params();
                // the first, the last and a spread of units
                let n = st.units.len();
                for i in (0..n).step_by((n / 16).max(1)).chain(n.checked_sub(1)) {
                    let _ = f.read_unit(&d, s, i);
                }
            }
        }
    }
}
