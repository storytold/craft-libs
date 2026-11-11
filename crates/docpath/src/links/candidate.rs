//! `check_candidate` and `rank`: whether a file may replace a missing link, and the order the
//! relink dialog lists candidates in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Link, LinkFiles, LinkIdentity};
use crate::names::natural_cmp;

/// Which properties a candidate must share with the missing file (the relink dialog's "Match
/// file properties" checks the library knows). An app's own properties (duration, timecode, frame
/// size) are its `RelinkCriterion`s, checked by its `CandidateChecker` after [`check_candidate`].
/// Serde with defaults for missing fields, so an app keeps the user's last choices in its
/// settings (FilmCraft stores its `MatchOptions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchCriteria {
    /// The file name without its extension, compared ignoring case. Default on.
    pub file_name: bool,
    /// The extension, compared ignoring case. Default on.
    pub extension: bool,
    /// The identity recorded at link time (FilmCraft's "Clip ID"): size and fingerprint. Links
    /// that recorded none skip it. Default on.
    pub same_file: bool,
}

impl Default for MatchCriteria {
    fn default() -> Self {
        // filmcraft: crates/engine/src/relink.rs:161 (`MatchOptions::default`: name, extension, clip id on).
        Self { file_name: true, extension: true, same_file: true }
    }
}

/// Why a candidate does not pass. Typed, so the dialog words each one (`RelinkLabels`) and an
/// app's checker adds its own words as [`Other`](CandidateProblem::Other).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CandidateProblem {
    /// The name without extension differs: `found` against `wanted`.
    NameDiffers {
        /// The candidate's name without extension.
        found: String,
        /// The name the link needs.
        wanted: String,
    },
    /// The extension differs.
    ExtensionDiffers {
        /// The candidate's extension.
        found: String,
        /// The extension the link needs.
        wanted: String,
    },
    /// Not the same file: the size differs (`found` and `wanted` in bytes), or, with `sizes`
    /// equal, the fingerprint differs.
    DifferentFile {
        /// The candidate's size in bytes, or its fingerprint when the sizes are equal.
        found: u64,
        /// The linked file's size in bytes, or its fingerprint when the sizes are equal.
        wanted: u64,
    },
    /// The file couldn't be read; the system's message.
    Unreadable(String),
    /// A problem the app's checker found, already in the user's language.
    Other(String),
}

/// One file checked as the new file of a missing link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The file.
    pub path: PathBuf,
    /// Passes every enabled check.
    pub ok: bool,
    /// Whether its identity matches the one recorded at link time; `None` when the link recorded
    /// none or the file couldn't be read.
    pub same_file: Option<bool>,
    /// Why it doesn't pass, in check order; empty when `ok`.
    pub problems: Vec<CandidateProblem>,
    /// The candidate's identity, read during the check, for the app to store when it relinks.
    pub identity: Option<LinkIdentity>,
    /// Ranking, higher first: same file 8, same name 4, same extension 2, passes 1 (FilmCraft's
    /// weights).
    pub score: u32,
}

/// Checks `path` as the new file of `link` by `criteria`: the name and extension against the
/// last component of `link.stored` (both separators count), ignoring ASCII case, each adding its
/// problem when its criterion is on; then [`LinkFiles::identity`] (an error adds `Unreadable` and
/// ends the check); then, when the link recorded an identity, `same_file` = size and fingerprint
/// equal, adding `DifferentFile` when `criteria.same_file` is on and they differ. `ok` is no
/// problems; `score` as [`Candidate::score`] says. Reads the file: call it in a job.
pub fn check_candidate<K>(link: &Link<K>, path: &Path, criteria: &MatchCriteria, files: &dyn LinkFiles) -> Candidate {
    // filmcraft: crates/engine/src/relink.rs:202 (`check_candidate`; the media checks stay FilmCraft's).
    let (wanted_name, wanted_ext) = split_ext(last_component(&link.stored));
    let found_file = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let (found_name, found_ext) = split_ext(&found_file);
    let name_ok = wanted_name.eq_ignore_ascii_case(found_name);
    let ext_ok = wanted_ext.eq_ignore_ascii_case(found_ext);
    let mut c = Candidate { path: path.to_path_buf(), ok: false, same_file: None, problems: Vec::new(), identity: None, score: 0 };
    if criteria.file_name && !name_ok {
        c.problems.push(CandidateProblem::NameDiffers { found: found_name.to_string(), wanted: wanted_name.to_string() });
    }
    if criteria.extension && !ext_ok {
        c.problems.push(CandidateProblem::ExtensionDiffers { found: found_ext.to_string(), wanted: wanted_ext.to_string() });
    }
    match files.identity(path) {
        Ok(found) => {
            c.identity = Some(found);
            if let Some(wanted) = link.identity {
                let same = wanted.size == found.size && wanted.fingerprint == found.fingerprint;
                c.same_file = Some(same);
                if criteria.same_file && !same {
                    c.problems.push(CandidateProblem::DifferentFile { found: found.size, wanted: wanted.size });
                }
            }
        }
        Err(e) => {
            c.problems.push(CandidateProblem::Unreadable(e.to_string()));
            c.score = score(&c, name_ok, ext_ok);
            return c;
        }
    }
    c.ok = c.problems.is_empty();
    c.score = score(&c, name_ok, ext_ok);
    c
}

/// Sorts `candidates` best first: by `score` descending, then by path with
/// `natural`-order file names (ties keep their search order). A candidate that passes and is
/// the same file is always first.
pub fn rank(candidates: &mut [Candidate]) {
    // filmcraft: crates/engine/src/relink.rs:281 (`score`: identity first, then name, extension).
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy())));
}

/// The ranking weights: same file 8, same name 4, same extension 2, passes 1.
fn score(c: &Candidate, name_ok: bool, ext_ok: bool) -> u32 {
    u32::from(c.same_file == Some(true)) * 8 + u32::from(name_ok) * 4 + u32::from(ext_ok) * 2 + u32::from(c.ok)
}

/// The text after the last `/` or `\`.
pub(super) fn last_component(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A file name split at its last dot into stem and extension; a leading dot is part of the stem.
pub(super) fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}
