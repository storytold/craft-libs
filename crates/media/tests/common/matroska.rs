//! Matroska fixtures and ffprobe helpers for `oracle_matroska.rs`. Tool discovery and the
//! fixture directory come from `common`.
#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// (ffmpeg, ffprobe), or `None` after printing `SKIPPED`.
pub fn tools() -> Option<(PathBuf, PathBuf)> {
    Some((crate::common::ffmpeg()?, crate::common::ffprobe()?))
}

pub fn fixture_dir() -> PathBuf {
    crate::common::fixture_subdir("matroska")
}

const V: &[&str] = &["-f", "lavfi", "-i", "testsrc2=size=160x120:rate=25"];
const A: &[&str] = &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"];

/// Subtitle inputs shared by fixture generators on other threads, so an existing file is never
/// truncated: it is left alone when current, else replaced by a rename.
fn write_subs(dir: &Path) {
    let srt = "1\n00:00:00,200 --> 00:00:00,900\nHello\n\n2\n00:00:01,000 --> 00:00:01,500\nWorld <i>two</i>\n\n";
    let vtt = "WEBVTT\n\n00:00:00.200 --> 00:00:00.900\nHello\n\n00:00:01.000 --> 00:00:01.500\nWorld\n\n";
    for (name, text) in [("in.srt", srt), ("in.vtt", vtt)] {
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|d| d == text.as_bytes()) {
            continue;
        }
        let tmp = crate::common::temp_path(&path);
        std::fs::write(&tmp, text).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
    }
}

/// Every fixture `spec`/`fixture` knows (for `generate_fixtures`).
pub const ALL_FIXTURES: &[&str] = &[
    "h264_aac.mkv",
    "vp9_opus.webm",
    "flac.mkv",
    "vorbis.mkv",
    "subs.mkv",
    "webvtt.webm",
    "cues_front.mkv",
    "hevc_hdr.mkv",
    "prores_pcm.mkv",
    "mjpeg_ac3.mkv",
    "av1.mkv",
    "live.mkv",
];

/// Fixture name → ffmpeg arguments (after `-y -v error`, before the output path).
/// `None` for fixtures written through a pipe (no Cues, unknown sizes).
pub fn spec(name: &str) -> Option<Vec<String>> {
    let cat = |parts: &[&[&str]]| parts.concat().into_iter().map(String::from).collect::<Vec<_>>();
    let x264: &[&str] = &["-c:v", "libx264", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p"];
    Some(match name {
        "h264_aac.mkv" => cat(&[V, A, &["-t", "2"], x264, &["-c:a", "aac"]]),
        "vp9_opus.webm" => cat(&[V, A, &["-t", "2", "-c:v", "libvpx-vp9", "-deadline", "realtime", "-g", "10", "-c:a", "libopus"]]),
        "flac.mkv" => cat(&[A, &["-t", "2", "-c:a", "flac"]]),
        "vorbis.mkv" => cat(&[A, &["-t", "2", "-ac", "2", "-c:a", "vorbis", "-strict", "-2"]]),
        "subs.mkv" => cat(&[
            V,
            &["-i", "in.srt", "-i", "in.srt", "-t", "2", "-map", "0:v", "-map", "1:s", "-map", "2:s"],
            x264,
            &["-c:s:0", "srt", "-c:s:1", "ass", "-metadata:s:s:0", "language=fre", "-metadata:s:s:1", "title=Styled"],
        ]),
        "webvtt.webm" => {
            cat(&[V, &["-i", "in.vtt", "-t", "2", "-map", "0:v", "-map", "1:s", "-c:v", "libvpx", "-deadline", "realtime", "-c:s", "webvtt"]])
        }
        "cues_front.mkv" => cat(&[V, A, &["-t", "3"], x264, &["-c:a", "aac", "-cues_to_front", "1", "-cluster_time_limit", "500"]]),
        "hevc_hdr.mkv" => cat(&[
            V,
            &[
                "-t",
                "1",
                "-c:v",
                "libx265",
                "-pix_fmt",
                "yuv420p10le",
                "-x265-params",
                "log-level=error:bframes=3:keyint=10:hdr10=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400",
                "-vf",
                "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc:range=tv",
            ],
        ]),
        "prores_pcm.mkv" => cat(&[V, A, &["-t", "1", "-c:v", "prores_ks", "-profile:v", "2", "-c:a", "pcm_s24le"]]),
        "mjpeg_ac3.mkv" => cat(&[V, A, &["-t", "1", "-c:v", "mjpeg", "-q:v", "5", "-c:a", "ac3", "-metadata", "title=craft-media test"]]),
        "av1.mkv" => cat(&[V, &["-t", "1", "-c:v", "libsvtav1", "-preset", "12", "-g", "10"]]),
        "live.mkv" => return None,
        _ => panic!("unknown fixture {name}"),
    })
}

/// Generate (once) and return the path of a fixture; `None` after printing `SKIPPED` if
/// generation failed (a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
pub fn fixture(ffmpeg: &Path, name: &str) -> Option<PathBuf> {
    let dir = fixture_dir();
    let out = dir.join(name);
    if out.exists() && std::fs::metadata(&out).map(|m| m.len() > 0).unwrap_or(false) {
        return Some(out);
    }
    write_subs(&dir);
    let tmp = crate::common::temp_path(&out);
    let mut c = Command::new(ffmpeg);
    c.current_dir(&dir).args(["-y", "-v", "error"]);
    match spec(name) {
        Some(args) => {
            let fmt = if name.ends_with(".webm") { "webm" } else { "matroska" };
            c.args(&args).args(["-f", fmt]).arg(&tmp);
        }
        None => {
            // streamed to a pipe: unknown-size Segment, no Cues, no Duration
            let f = std::fs::File::create(&tmp).unwrap();
            c.args(V).args(A);
            c.args(["-t", "2", "-c:v", "libx264", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p", "-c:a", "aac", "-f", "matroska", "pipe:1"]);
            c.stdout(Stdio::from(f));
        }
    }
    // the rename is atomic: concurrent generators race harmlessly
    crate::common::run_generator(&mut c, &tmp, &out, crate::common::oracles_required())
}

pub fn ffprobe_json(ffprobe: &Path, path: &Path) -> Value {
    let out = Command::new(ffprobe)
        .args(["-v", "error", "-show_packets", "-show_streams", "-show_format", "-show_chapters", "-of", "json"])
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert!(out.status.success(), "ffprobe failed on {}: {}", path.display(), String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).expect("ffprobe json")
}

pub fn int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}
