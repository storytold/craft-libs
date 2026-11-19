#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Oracle-test support for the craft-libs media crates (craft-media, craft-audio, and the video
//! crate to come): ffmpeg/ffprobe discovery, the skip-or-fail rule, and fixture locations.
//! ffmpeg is used only as an external fixture generator and oracle (never linked). Test support
//! only: unpublished, and used as a dev-dependency.
//!
//! Tool search order: `CRAFT_MEDIA_FFMPEG` / `CRAFT_MEDIA_FFPROBE` (an explicit path; a wrong
//! path fails the test), then `PATH`, then well-known install directories. A missing tool, a
//! failed fixture generator or a failed ffprobe run prints `SKIPPED` and the test passes, unless
//! `CRAFT_MEDIA_REQUIRE_ORACLES=1` is set: then each of these fails the test.
//! Fixtures are generated into `$CRAFT_MEDIA_FIXTURES_DIR` or `<workspace>/target/fixtures`, one
//! directory per crate or format, and never committed.
//!
//! [`hostile::hammer`] runs a parser over truncations and mutations of a valid input and fails on
//! a panic or a slow run; `CRAFT_MEDIA_HOSTILE_MULT=10` runs ten times the iterations.
//!
//! Failing loudly is what a test helper is for: a wrong `CRAFT_MEDIA_*` setting or an unwritable
//! fixture directory panics, which fails the test that asked.

pub mod hostile;

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

#[allow(clippy::panic)]
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

#[allow(clippy::panic)]
fn skip_with<T>(required: bool, reason: &str) -> Option<T> {
    assert!(!required, "oracle: {reason}, and {REQUIRE_ENV}=1 forbids skipping oracle tests");
    eprintln!("SKIPPED (oracle): {reason}");
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

/// `$CRAFT_MEDIA_FIXTURES_DIR/<name>` or `<workspace>/target/fixtures/<name>`, created if missing.
#[allow(clippy::unwrap_used)]
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

/// A new, empty directory in the system temporary directory that only this call created: made with
/// `create_dir`, which fails rather than follow a pre-planted name or symlink, so a test can write
/// fixed file names inside it. The caller removes it.
#[allow(clippy::panic)]
pub fn private_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for _ in 0..100 {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("craft-media-{tag}-{}-{nanos:x}-{n}", std::process::id()));
        match std::fs::create_dir(&d) {
            Ok(()) => return d,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => panic!("cannot create {}: {e}", d.display()),
        }
    }
    panic!("no free scratch directory name for {tag}");
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

/// Check if ffmpeg has a specific encoder compiled in.
pub fn ffmpeg_has_encoder(ff: &Path, encoder: &str) -> bool {
    let Ok(output) = Command::new(ff).args(["-hide_banner", "-encoders"]).output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    parse_has_encoder(&String::from_utf8_lossy(&output.stdout), encoder)
}

fn parse_has_encoder(stdout: &str, encoder: &str) -> bool {
    stdout.lines().any(|line| {
        let mut parts = line.split_whitespace();
        if let (Some(_flags), Some(name)) = (parts.next(), parts.next()) { name == encoder } else { false }
    })
}

/// Report a skipped test that is not an oracle check (an unsupported platform or an optional
/// encoder capability). Unlike [`skip`], this never fails under `CRAFT_MEDIA_REQUIRE_ORACLES=1`.
pub fn skip_test(what: &str, reason: &str) {
    eprintln!("SKIPPED ({what}): {reason}");
}

/// Find ffmpeg, or print a skip message and return None.
pub fn ffmpeg_or_skip(reason: &str) -> Option<PathBuf> {
    tool("ffmpeg").or_else(|| skip(&format!("{reason}: ffmpeg not found (set {} or put ffmpeg on PATH)", env_var("ffmpeg"))))
}

/// Log progress of long tests.
pub fn set_progress(label: &str) {
    eprintln!("PROGRESS: {label}");
}

/// Run closure with an execution deadline, terminating with a panic if the limit is exceeded.
#[allow(clippy::panic)]
pub fn with_deadline<F: FnOnce() + Send + 'static>(secs: u64, f: F) {
    let timeout_secs = std::env::var("CRAFT_TEST_TIMEOUT_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(secs);
    with_timeout(timeout_secs, f);
}

#[allow(clippy::panic)]
fn with_timeout<F: FnOnce() + Send + 'static>(timeout_secs: u64, f: F) {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        f();
        let _ = tx.send(());
    });
    match rx.recv_timeout(std::time::Duration::from_secs(timeout_secs)) {
        Ok(()) => {
            if let Err(payload) = handle.join() {
                std::panic::resume_unwind(payload);
            }
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("test exceeded execution deadline of {timeout_secs} seconds");
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(payload) = handle.join() {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

/// Return fixture directory for a crate/category, creating it if needed.
pub fn fixtures_dir(subdir: &str) -> PathBuf {
    fixture_subdir(subdir)
}

pub mod fixtures {
    use super::temp_path;
    use std::path::{Path, PathBuf};

    /// Atomically publish a fixture by renaming `tmp` to `path`.
    pub fn publish(tmp: &Path, path: &Path) -> std::io::Result<()> {
        std::fs::rename(tmp, path)
    }

    /// Generate fixture `out` using generator closure if it does not already exist. A generator
    /// that returns `false`, or whose output cannot be moved into place, skips (see
    /// [`skip`](super::skip)): a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`, so a required
    /// oracle run cannot pass with a fixture missing.
    pub fn generate<F: FnOnce(&Path) -> bool>(out: &Path, f: F) -> Option<PathBuf> {
        generate_with(out, super::oracles_required(), f)
    }

    /// [`generate`] with `required` explicit, so tests can set it without touching the process
    /// environment.
    pub fn generate_with<F: FnOnce(&Path) -> bool>(out: &Path, required: bool, f: F) -> Option<PathBuf> {
        if out.exists() {
            return Some(out.to_path_buf());
        }
        let tmp = temp_path(out);
        let reason = if f(&tmp) {
            match std::fs::rename(&tmp, out) {
                Ok(()) => return Some(out.to_path_buf()),
                Err(e) => format!("could not move {} into place: {e}", tmp.display()),
            }
        } else {
            format!("fixture generator failed for {}", out.display())
        };
        let _ = std::fs::remove_file(&tmp);
        super::skip_with(required, &reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_generator_skips_when_optional() {
        let dir = private_dir("testkit-gen-opt");
        let out = dir.join("fixture");
        assert!(fixtures::generate_with(&out, false, |_| false).is_none());
        assert!(!out.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[should_panic(expected = "forbids skipping oracle tests")]
    fn failed_generator_fails_when_required() {
        let dir = private_dir("testkit-gen-req");
        let out = dir.join("fixture");
        let result = std::panic::catch_unwind(|| fixtures::generate_with(&out, true, |_| false));
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    #[test]
    fn parse_has_encoder_exact_match() {
        let sample = "\
Encoders:
 V..... = Video
 ------
 V....D libvpx-vp9          libvpx VP9 (codec vp9)
 V....D libvpx              libvpx VP8 (codec vp8)
 A....D flac                FLAC (Free Lossless Audio Codec)
";
        assert!(parse_has_encoder(sample, "libvpx"));
        assert!(parse_has_encoder(sample, "libvpx-vp9"));
        assert!(parse_has_encoder(sample, "flac"));
        assert!(!parse_has_encoder(sample, "vp8"));
        assert!(!parse_has_encoder(sample, "libvp"));
    }

    #[test]
    fn with_deadline_completes() {
        with_deadline(5, || {
            // fast operation
        });
    }

    #[test]
    #[should_panic(expected = "test exceeded execution deadline")]
    fn with_deadline_times_out() {
        with_timeout(1, || {
            std::thread::sleep(std::time::Duration::from_secs(3));
        });
    }
}
