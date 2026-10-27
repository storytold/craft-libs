//! Shared helpers for oracle tests: ffmpeg/ffprobe discovery and fixture generation.
//! ffmpeg is used only as an external fixture generator and oracle (never linked).
//!
//! Tool search order: `CRAFT_MEDIA_FFMPEG` / `CRAFT_MEDIA_FFPROBE` (an explicit path; a wrong
//! path fails the test), then `PATH`, then well-known install directories. A missing tool, a
//! failed fixture generator or a failed ffprobe run prints `SKIPPED` and the test passes, unless
//! `CRAFT_MEDIA_REQUIRE_ORACLES=1` is set: then each of these fails the test.
//! Fixtures are generated into `$CRAFT_MEDIA_FIXTURES_DIR` or `<workspace>/target/fixtures`, one
//! directory per codec family, and never committed. The environment variables are the ones
//! craft-media's oracle tests use, so one setting covers both crates.
#![allow(dead_code)]

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
    assert!(!required, "craft-audio oracle: {reason}, and {REQUIRE_ENV}=1 forbids skipping oracle tests");
    eprintln!("SKIPPED (craft-audio oracle): {reason}");
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
