//! Shared test helpers and ffmpeg oracle discovery for Craft media codecs.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

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

fn find_tool(name: &str) -> Option<PathBuf> {
    let env_name = format!("CRAFT_{}_PATH", name.to_ascii_uppercase());
    if let Some(e) = std::env::var_os(&env_name).filter(|e| !e.is_empty()) {
        let p = PathBuf::from(e);
        if p.is_file() {
            return Some(p);
        }
    }
    let fallback_env = format!("CRAFT_MEDIA_{}", name.to_ascii_uppercase());
    if let Some(e) = std::env::var_os(&fallback_env).filter(|e| !e.is_empty()) {
        let p = PathBuf::from(e);
        if p.is_file() {
            return Some(p);
        }
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

/// Find ffmpeg binary if present.
pub fn ffmpeg() -> Option<PathBuf> {
    static FFMPEG: OnceLock<Option<PathBuf>> = OnceLock::new();
    FFMPEG.get_or_init(|| find_tool("ffmpeg")).clone()
}

/// Find ffmpeg, or print a skip message and return None.
pub fn ffmpeg_or_skip(reason: &str) -> Option<PathBuf> {
    let ff = ffmpeg();
    if ff.is_none() {
        skip(reason, "ffmpeg not found");
    }
    ff
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

/// Report a skipped test.
pub fn skip(what: &str, reason: &str) {
    eprintln!("SKIPPED ({what}): {reason}");
}

/// Log progress of long tests.
pub fn set_progress(label: &str) {
    eprintln!("PROGRESS: {label}");
}

/// Run closure with an execution deadline, terminating with a panic if the limit is exceeded.
#[allow(clippy::panic)]
pub fn with_deadline<F: FnOnce() + Send + 'static>(secs: u64, f: F) {
    let timeout_secs = std::env::var("CRAFT_TEST_TIMEOUT_SECS")
        .or_else(|_| std::env::var("FILMCRAFT_HOSTILE_TIMEOUT_SECS"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(secs);

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
    let base = match std::env::var_os("CRAFT_FIXTURES_DIR").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let here = Path::new(env!("CARGO_MANIFEST_DIR"));
            let root = here.parent().and_then(Path::parent).unwrap_or(here);
            root.join("target").join("fixtures")
        }
    };
    let dir = base.join(subdir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Generate a unique temporary path sibling to `path`.
pub fn temp_path(path: &Path) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let tid = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
    let n = N.fetch_add(1, Ordering::Relaxed);
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem}.tmp-{}-{tid}-{n}.{}", std::process::id(), ext.to_string_lossy()),
        None => format!("{stem}.tmp-{}-{tid}-{n}", std::process::id()),
    };
    path.with_file_name(name)
}

pub mod fixtures {
    use super::temp_path;
    use std::path::{Path, PathBuf};

    /// Atomically publish a fixture by renaming `tmp` to `path`.
    pub fn publish(tmp: &Path, path: &Path) -> std::io::Result<()> {
        std::fs::rename(tmp, path)
    }

    /// Generate fixture `out` using generator closure if it does not already exist.
    pub fn generate<F: FnOnce(&Path) -> bool>(out: &Path, f: F) -> Option<PathBuf> {
        if out.exists() {
            return Some(out.to_path_buf());
        }
        let tmp = temp_path(out);
        if f(&tmp) {
            match std::fs::rename(&tmp, out) {
                Ok(()) => Some(out.to_path_buf()),
                Err(_) => {
                    let _ = std::fs::remove_file(&tmp);
                    None
                }
            }
        } else {
            let _ = std::fs::remove_file(&tmp);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        with_deadline(1, || {
            std::thread::sleep(std::time::Duration::from_secs(3));
        });
    }
}
