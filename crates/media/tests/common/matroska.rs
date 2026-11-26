//! Matroska fixtures and ffprobe helpers for `oracle_matroska.rs`. Tool discovery and the
//! fixture directory come from `common`.
#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// (ffmpeg, ffprobe), or `None` after printing `SKIPPED`.
pub fn tools() -> Option<(PathBuf, PathBuf)> {
    Some((crate::integration::common::ffmpeg()?, crate::integration::common::ffprobe()?))
}

pub fn fixture_dir() -> PathBuf {
    crate::integration::common::fixture_subdir("matroska")
}

const V: &[&str] = &["-f", "lavfi", "-i", "testsrc2=size=160x120:rate=25"];
const A: &[&str] = &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"];

/// Every fixture `spec`/`fixture` knows (for `generate_fixtures`).
pub const ALL_FIXTURES: &[&str] = &["h264_aac.mkv", "flac.mkv", "vorbis.mkv", "prores_pcm.mkv"];

/// Fixture name → ffmpeg arguments (after `-y -v error`, before the output path).
pub fn spec(name: &str) -> Vec<String> {
    let cat = |parts: &[&[&str]]| parts.concat().into_iter().map(String::from).collect::<Vec<_>>();
    match name {
        "h264_aac.mkv" => cat(&[V, A, &["-t", "2", "-c:v", "libx264", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p", "-c:a", "aac"]]),
        "flac.mkv" => cat(&[A, &["-t", "2", "-c:a", "flac"]]),
        "vorbis.mkv" => cat(&[A, &["-t", "2", "-ac", "2", "-c:a", "vorbis", "-strict", "-2"]]),
        "prores_pcm.mkv" => cat(&[V, A, &["-t", "1", "-c:v", "prores_ks", "-profile:v", "2", "-c:a", "pcm_s24le"]]),
        _ => panic!("unknown fixture {name}"),
    }
}

/// Generate (once) and return the path of a fixture; `None` after printing `SKIPPED` if
/// generation failed (a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
pub fn fixture(ffmpeg: &Path, name: &str) -> Option<PathBuf> {
    let dir = fixture_dir();
    let out = dir.join(name);
    if out.exists() && std::fs::metadata(&out).map(|m| m.len() > 0).unwrap_or(false) {
        return Some(out);
    }
    let tmp = crate::integration::common::temp_path(&out);
    let mut c = Command::new(ffmpeg);
    c.current_dir(&dir).args(["-y", "-v", "error"]).args(spec(name)).args(["-f", "matroska"]).arg(&tmp);
    // the rename is atomic: concurrent generators race harmlessly
    crate::integration::common::run_generator(&mut c, &tmp, &out, crate::integration::common::oracles_required())
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
