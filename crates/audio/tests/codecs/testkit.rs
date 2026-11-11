//! The parts of FilmCraft's `filmcraft-testkit` that the ported codec tests use, on craft-media's
//! oracle rules (`tests/common`): `CRAFT_MEDIA_FFMPEG`, then `PATH`; a missing tool skips unless
//! `CRAFT_MEDIA_REQUIRE_ORACLES=1`; fixtures go to `$CRAFT_MEDIA_FIXTURES_DIR` or
//! `target/fixtures`, never committed.

use crate::common;
use std::path::{Path, PathBuf};

/// `let ff = require_ffmpeg!();`: the ffmpeg path, or return from the test after printing
/// `SKIPPED` (a failure with `CRAFT_MEDIA_REQUIRE_ORACLES=1`).
macro_rules! require_ffmpeg {
    () => {
        match $crate::testkit::ffmpeg_or_skip(concat!(module_path!(), ":", line!())) {
            Some(p) => p,
            None => return,
        }
    };
    ($ret:expr) => {
        match $crate::testkit::ffmpeg_or_skip(concat!(module_path!(), ":", line!())) {
            Some(p) => p,
            None => return $ret,
        }
    };
}

/// ffmpeg, or `None` after printing `SKIPPED` for `context`.
pub fn ffmpeg_or_skip(context: &str) -> Option<PathBuf> {
    common::tool("ffmpeg").or_else(|| common::skip(&format!("{context}: ffmpeg not found (set CRAFT_MEDIA_FFMPEG or put ffmpeg on PATH)")))
}

/// `<fixtures>/<sub>` (`sub` may hold `/`), created if missing.
pub fn fixtures_dir(sub: &str) -> PathBuf {
    let mut d = common::fixture_subdir("codecs");
    for part in sub.split('/').filter(|p| !p.is_empty()) {
        d.push(part);
    }
    std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("create {}: {e}", d.display()));
    d
}

pub mod fixtures {
    use super::*;

    /// `out` if it already exists (non-empty); otherwise `make(tmp)` on a temporary sibling, renamed
    /// into place. `None` when `make` fails.
    pub fn generate(out: &Path, make: impl FnOnce(&Path) -> bool) -> Option<PathBuf> {
        if std::fs::metadata(out).is_ok_and(|m| m.len() > 0) {
            return Some(out.to_path_buf());
        }
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = common::temp_path(out);
        if !make(&tmp) || !tmp.exists() {
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
        std::fs::rename(&tmp, out).ok()?;
        Some(out.to_path_buf())
    }
}
