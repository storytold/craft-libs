//! craft-media's oracle-test helpers: the generic ones from `craft-media-testkit`, and the MP4
//! fixtures and ffprobe queries the container tests share.
#![allow(dead_code)]

pub use craft_media_testkit::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixtures of the `isobmff` tests.
pub fn fixture_dir() -> PathBuf {
    fixture_subdir("isobmff")
}

pub fn out_dir() -> PathBuf {
    let d = fixture_dir().join("out");
    std::fs::create_dir_all(&d).unwrap();
    d
}

const V: &[&str] = &["-f", "lavfi", "-i", "testsrc2=size=160x120:rate=25"];
const A: &[&str] = &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"];

/// (fixture name, ffmpeg arguments after the global options, output file name derived from the name).
pub fn fixture_specs() -> Vec<(&'static str, Vec<&'static str>)> {
    let cat = |parts: &[&[&'static str]]| parts.concat();
    vec![
        ("h264_bframes.mp4", cat(&[V, A, &["-t", "2", "-c:v", "libx264", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p", "-c:a", "aac"]])),
        (
            "frag.mp4",
            cat(&[
                V,
                A,
                &[
                    "-t",
                    "3",
                    "-c:v",
                    "libx264",
                    "-bf",
                    "2",
                    "-g",
                    "25",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-movflags",
                    "frag_keyframe+empty_moov",
                ],
            ]),
        ),
        (
            "tmcd_2997df.mov",
            vec!["-f", "lavfi", "-i", "testsrc2=size=160x120:rate=30000/1001", "-t", "1", "-c:v", "mjpeg", "-timecode", "01:00:00;00"],
        ),
    ]
}

fn run_ffmpeg(ff: &Path, args: &[&str], out: &Path) -> Option<PathBuf> {
    let tmp = temp_path(out);
    let mut cmd = Command::new(ff);
    cmd.args(["-v", "error", "-y", "-nostdin"]).args(args).arg(&tmp);
    run_generator(&mut cmd, &tmp, out, oracles_required())
}

/// Path to a fixture, generating it with ffmpeg if needed. `None` after printing `SKIPPED` if
/// ffmpeg is missing or the encoder isn't available (a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
pub fn fixture(name: &str) -> Option<PathBuf> {
    let ff = ffmpeg()?;
    let path = fixture_dir().join(name);
    if path.exists() {
        return Some(path);
    }
    let (_, args) = fixture_specs().into_iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("unknown fixture {name}"));
    run_ffmpeg(&ff, &args, &path)
}

pub fn all_fixture_names() -> Vec<&'static str> {
    fixture_specs().iter().map(|(n, _)| *n).collect()
}

/// ffprobe's JSON for `path`, or `None` after printing `SKIPPED` (a failure with
/// `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
pub fn probe_json(path: &Path, what: &[&str]) -> Option<Value> {
    let fp = ffprobe()?;
    let o = match Command::new(fp).args(["-v", "error", "-of", "json"]).args(what).arg(path).output() {
        Ok(o) if o.status.success() => o,
        Ok(o) => return skip(&format!("ffprobe failed on {}: {}", path.display(), String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => return skip(&format!("could not run ffprobe: {e}")),
    };
    match serde_json::from_slice(&o.stdout) {
        Ok(v) => Some(v),
        Err(e) => skip(&format!("ffprobe output for {} is not JSON: {e}", path.display())),
    }
}

/// ffprobe's packets for `path`, or `None` after a skipped probe (see [`probe_json`]).
pub fn packets(path: &Path) -> Option<Vec<Value>> {
    probe_json(path, &["-show_packets"]).map(|v| v["packets"].as_array().cloned().unwrap_or_default())
}

/// ffprobe's streams for `path`, or `None` after a skipped probe (see [`probe_json`]).
pub fn streams(path: &Path) -> Option<Vec<Value>> {
    probe_json(path, &["-show_streams"]).map(|v| v["streams"].as_array().cloned().unwrap_or_default())
}

pub fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Decode a file with ffmpeg to the null muxer; returns stderr (empty on success).
pub fn ffmpeg_decode_errors(path: &Path) -> Option<String> {
    let ff = ffmpeg()?;
    let o = Command::new(ff).args(["-v", "error", "-nostdin", "-i"]).arg(path).args(["-f", "null", "-"]).output().ok()?;
    let mut s = String::from_utf8_lossy(&o.stderr).into_owned();
    if !o.status.success() {
        s.push_str("\n(ffmpeg exited with failure)");
    }
    Some(s)
}
