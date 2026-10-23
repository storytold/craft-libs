//! Shared helpers for oracle tests: ffmpeg/ffprobe discovery and fixture generation.
//! ffmpeg is used only as an external fixture generator and oracle (never linked).
//!
//! Tool search order: `CRAFT_MEDIA_FFMPEG` / `CRAFT_MEDIA_FFPROBE` (an explicit path; a wrong
//! path fails the test), then `PATH`, then well-known install directories. A missing tool, a
//! failed fixture generator or a failed ffprobe run prints `SKIPPED` and the test passes, unless
//! `CRAFT_MEDIA_REQUIRE_ORACLES=1` is set: then each of these fails the test.
//! Fixtures are generated into `$CRAFT_MEDIA_FIXTURES_DIR` or `<workspace>/target/fixtures`, one
//! directory per container (`isobmff`, `matroska`), and never committed.
#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

const REQUIRE_ENV: &str = "CRAFT_MEDIA_REQUIRE_ORACLES";

const KNOWN_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/opt/local/bin",
    "/snap/bin",
    r"C:\ffmpeg\bin",
    r"C:\Program Files\ffmpeg\bin",
    r"C:\ProgramData\chocolatey\bin",
];

fn env_var(name: &str) -> String {
    format!("CRAFT_MEDIA_{}", name.to_ascii_uppercase())
}

fn find_tool(name: &str) -> Option<PathBuf> {
    if let Some(e) = std::env::var_os(env_var(name)).filter(|e| !e.is_empty()) {
        let p = PathBuf::from(e);
        assert!(p.is_file(), "{} is set but `{}` is not a file", env_var(name), p.display());
        return Some(p);
    }
    let names = if cfg!(windows) { [format!("{name}.exe"), name.to_string()] } else { [name.to_string(), format!("{name}.exe")] };
    let path_dirs = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let dirs = path_dirs.into_iter().chain(KNOWN_DIRS.iter().map(PathBuf::from));
    for dir in dirs {
        for n in &names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Find a tool silently (cached for the process for ffmpeg and ffprobe).
pub fn tool(name: &str) -> Option<PathBuf> {
    static FFMPEG: OnceLock<Option<PathBuf>> = OnceLock::new();
    static FFPROBE: OnceLock<Option<PathBuf>> = OnceLock::new();
    match name {
        "ffmpeg" => FFMPEG.get_or_init(|| find_tool("ffmpeg")).clone(),
        "ffprobe" => FFPROBE.get_or_init(|| find_tool("ffprobe")).clone(),
        _ => find_tool(name),
    }
}

pub fn oracles_required() -> bool {
    std::env::var(REQUIRE_ENV).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// Give up on an oracle check: print `SKIPPED` and return `None`, or fail when oracles are
/// required.
pub fn skip<T>(reason: &str) -> Option<T> {
    skip_with(oracles_required(), reason)
}

fn skip_with<T>(required: bool, reason: &str) -> Option<T> {
    assert!(!required, "craft-media oracle: {reason}, and {REQUIRE_ENV}=1 forbids skipping oracle tests");
    eprintln!("SKIPPED (craft-media oracle): {reason}");
    None
}

/// Run a fixture generator writing `tmp`, then rename it to `out`. A generator that cannot start
/// or fails skips (see [`skip`]); `required` is explicit so tests can set it without touching
/// the process environment.
pub fn run_generator(cmd: &mut Command, tmp: &Path, out: &Path, required: bool) -> Option<PathBuf> {
    let reason = match cmd.output() {
        Ok(o) if o.status.success() => match std::fs::rename(tmp, out) {
            Ok(()) => return Some(out.to_path_buf()),
            Err(e) => format!("could not move {} into place: {e}", tmp.display()),
        },
        Ok(o) => format!("generator failed for {} ({}): {}", out.display(), o.status, String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => format!("could not start generator for {}: {e}", out.display()),
    };
    let _ = std::fs::remove_file(tmp);
    skip_with(required, &format!("{reason} (encoder unavailable?)"))
}

fn tool_or_skip(name: &str) -> Option<PathBuf> {
    tool(name).or_else(|| skip(&format!("{name} not found (set {} or put {name} on PATH)", env_var(name))))
}

/// ffmpeg, or `None` after printing `SKIPPED` (a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
pub fn ffmpeg() -> Option<PathBuf> {
    tool_or_skip("ffmpeg")
}

/// ffprobe, or `None` after printing `SKIPPED` (a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
pub fn ffprobe() -> Option<PathBuf> {
    tool_or_skip("ffprobe")
}

/// Fixtures of the `isobmff` tests.
pub fn fixture_dir() -> PathBuf {
    fixture_subdir("isobmff")
}

/// `$CRAFT_MEDIA_FIXTURES_DIR/<name>` or `<workspace>/target/fixtures/<name>`, created if missing.
pub fn fixture_subdir(name: &str) -> PathBuf {
    let d = match std::env::var_os("CRAFT_MEDIA_FIXTURES_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let here = Path::new(env!("CARGO_MANIFEST_DIR"));
            let root = here.parent().and_then(Path::parent).unwrap_or(here);
            root.join("target").join("fixtures")
        }
    }
    .join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn out_dir() -> PathBuf {
    let d = fixture_dir().join("out");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A temporary sibling of `out` that no other thread or process will pick; the extension is kept
/// because ffmpeg chooses the muxer from it.
pub fn temp_path(out: &Path) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let tid = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
    let n = N.fetch_add(1, Ordering::Relaxed);
    let stem = out.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let name = match out.extension() {
        Some(ext) => format!("{stem}.tmp-{}-{tid}-{n}.{}", std::process::id(), ext.to_string_lossy()),
        None => format!("{stem}.tmp-{}-{tid}-{n}", std::process::id()),
    };
    out.with_file_name(name)
}

/// Run a fixture generator and print `FIXTURE <made|cached|skipped> <name>`.
pub fn generate_and_report<T>(name: &str, outputs: &[PathBuf], generate: impl FnOnce() -> Option<T>) -> Option<T> {
    let existed = !outputs.is_empty() && outputs.iter().all(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0));
    let r = generate();
    let status = match (&r, existed) {
        (None, _) => "skipped",
        (Some(_), true) => "cached",
        (Some(_), false) => "made",
    };
    println!("FIXTURE {status} {name}");
    r
}

const V: &[&str] = &["-f", "lavfi", "-i", "testsrc2=size=160x120:rate=25"];
const A: &[&str] = &["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"];

/// (fixture name, ffmpeg arguments after the global options, output file name derived from the name).
pub fn fixture_specs() -> Vec<(&'static str, Vec<&'static str>)> {
    let cat = |parts: &[&[&'static str]]| parts.concat();
    vec![
        ("h264_bframes.mp4", cat(&[V, A, &["-t", "2", "-c:v", "libx264", "-bf", "2", "-g", "12", "-pix_fmt", "yuv420p", "-c:a", "aac"]])),
        ("h264.mov", cat(&[V, A, &["-t", "2", "-c:v", "libx264", "-bf", "3", "-g", "25", "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le"]])),
        (
            "hevc.mp4",
            cat(&[V, &["-t", "2", "-c:v", "libx265", "-tag:v", "hvc1", "-x265-params", "log-level=error:bframes=3", "-pix_fmt", "yuv420p"]]),
        ),
        ("prores.mov", cat(&[V, A, &["-t", "1", "-c:v", "prores_ks", "-profile:v", "3", "-c:a", "pcm_s24le"]])),
        ("prores4444.mov", cat(&[V, &["-t", "0.5", "-c:v", "prores_ks", "-profile:v", "4", "-pix_fmt", "yuva444p10le"]])),
        ("mjpeg.mov", cat(&[V, &["-t", "1", "-c:v", "mjpeg", "-q:v", "5"]])),
        (
            "dnxhd.mov",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=25",
                "-t",
                "0.2",
                "-c:v",
                "dnxhd",
                "-profile:v",
                "dnxhr_lb",
                "-pix_fmt",
                "yuv422p",
            ],
        ),
        ("pcm_f32.mov", cat(&[A, &["-t", "1", "-c:a", "pcm_f32le"]])),
        ("pcm_s16be.mov", cat(&[A, &["-t", "1", "-c:a", "pcm_s16be"]])),
        ("aac.mp4", cat(&[A, &["-t", "2", "-c:a", "aac", "-b:a", "96k"]])),
        ("aac_stereo44.m4a", vec!["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100", "-t", "1.5", "-ac", "2", "-c:a", "aac"]),
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
            "frag_dash.mp4",
            cat(&[
                V,
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
                    "-movflags",
                    "frag_keyframe+empty_moov+default_base_moof",
                ],
            ]),
        ),
        ("faststart.mp4", cat(&[V, A, &["-t", "2", "-c:v", "libx264", "-bf", "2", "-pix_fmt", "yuv420p", "-c:a", "aac", "-movflags", "+faststart"]])),
        (
            "tmcd_2997df.mov",
            vec!["-f", "lavfi", "-i", "testsrc2=size=160x120:rate=30000/1001", "-t", "1", "-c:v", "mjpeg", "-timecode", "01:00:00;00"],
        ),
        (
            "tmcd_23976.mov",
            vec![
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x120:rate=24000/1001",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-timecode",
                "01:00:00:00",
            ],
        ),
        (
            "h264_2997.mp4",
            vec!["-f", "lavfi", "-i", "testsrc2=size=160x120:rate=30000/1001", "-t", "2", "-c:v", "libx264", "-bf", "2", "-pix_fmt", "yuv420p"],
        ),
        ("flac.mp4", cat(&[A, &["-t", "1", "-c:a", "flac"]])),
        ("alac.m4a", cat(&[A, &["-t", "1", "-c:a", "alac"]])),
        ("opus.mp4", cat(&[A, &["-t", "1", "-c:a", "libopus"]])),
        ("ac3.mp4", cat(&[A, &["-t", "1", "-c:a", "ac3"]])),
        ("vp9.mp4", cat(&[V, &["-t", "1", "-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8"]])),
        ("av1.mp4", cat(&[V, &["-t", "1", "-c:v", "libsvtav1", "-preset", "12"]])),
        (
            "colr_pasp.mov",
            cat(&[
                V,
                &[
                    "-t",
                    "0.5",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-color_primaries",
                    "bt709",
                    "-color_trc",
                    "bt709",
                    "-colorspace",
                    "bt709",
                    "-aspect",
                    "16:9",
                ],
            ]),
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
    if name == "editlist_cut.mp4" {
        let src = fixture("h264_bframes.mp4")?;
        let s = src.to_str().expect("fixture path is UTF-8").to_string();
        return run_ffmpeg(&ff, &["-ss", "0.5", "-i", &s, "-c", "copy"], &path);
    }
    let (_, args) = fixture_specs().into_iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("unknown fixture {name}"));
    run_ffmpeg(&ff, &args, &path)
}

pub fn all_fixture_names() -> Vec<&'static str> {
    let mut v: Vec<&str> = fixture_specs().iter().map(|(n, _)| *n).collect();
    v.push("editlist_cut.mp4");
    v
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
