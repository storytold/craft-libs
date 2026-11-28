//! Folder remaps (one relink carries to the other missing links) and Relink to Folder.

use std::path::{Path, PathBuf};

use super::candidate::last_component;
use super::{Link, LinkFiles};

/// The folder change one relink implies: links under `from` move under `to`. Separators are `/`
/// in both, so a remap derived from a Windows path applies to a macOS one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remap {
    /// The old folder prefix, `/`-separated, without a trailing `/`.
    pub from: String,
    /// The new folder prefix, `/`-separated, without a trailing `/`.
    pub to: String,
}

impl Remap {
    /// `path` under the new prefix: `None` when `path` (with `\` read as `/`) doesn't start with
    /// `from` followed by `/` or its end. The result uses `/`; callers pass it through
    /// [`resolve_absolute`](crate::resolve_absolute) before they check the file.
    pub fn apply(&self, path: &str) -> Option<String> {
        // filmcraft: crates/engine/src/relink.rs:306 (`apply_remap`).
        let path = path.replace('\\', "/");
        let rest = path.strip_prefix(self.from.trim_end_matches('/'))?;
        if !(rest.is_empty() || rest.starts_with('/')) {
            return None;
        }
        Some(format!("{}{rest}", self.to.trim_end_matches('/')))
    }
}

/// The remap implied by a file moving from `old` to `new` (stored paths): both are read with `\`
/// as `/`, the longest run of equal trailing components is dropped, and the rest of each is the
/// prefix. `None` when no trailing component is equal or the prefixes are equal.
/// `C:\shoot\day1\a.mov` → `/Volumes/T7/shoot/day1/a.mov` gives `C:` → `/Volumes/T7`.
pub fn derive_remap(old: &str, new: &str) -> Option<Remap> {
    // filmcraft: crates/engine/src/relink.rs:288 (`derive_remap`).
    let (old, new) = (old.replace('\\', "/"), new.replace('\\', "/"));
    let old_parts: Vec<&str> = old.split('/').collect();
    let new_parts: Vec<&str> = new.split('/').collect();
    let equal = old_parts.iter().rev().zip(new_parts.iter().rev()).take_while(|(a, b)| a == b).count();
    if equal == 0 {
        return None;
    }
    let from = old_parts[..old_parts.len() - equal].join("/");
    let to = new_parts[..new_parts.len() - equal].join("/");
    (from != to).then_some(Remap { from, to })
}

/// Where `link`'s file would be in `folder`: its stored file name (the last component, `/` or
/// `\`), with the extension replaced by `extension` when given (no leading dot), joined to
/// `folder`. Lexical, no I/O. `None` unless the name is a plain file name: no separators, no NUL,
/// not `.` or `..`, and [`classify`](crate::classify) says `Local` with no drive or scheme (so
/// `a\b`, `file:%2Ftmp%2Fx.png`, `C:x.png`, `NUL` give `None`).
pub fn folder_match<K>(link: &Link<K>, folder: &Path, extension: Option<&str>) -> Option<PathBuf> {
    // designcraft: crates/engine/src/cmd/links.rs:474 (`relink_folder`: plain name directly in the folder).
    let name = last_component(&link.stored);
    if extension.is_some_and(|e| e.contains(['/', '\\', '\0'])) {
        return None;
    }
    // A name that resolves to another path (a `file:` URL, a drive-relative `C:x.png`) is not a
    // plain name even though it holds no separator.
    let plain = crate::names::is_plain_file_name(name)
        && !name.contains('\0')
        && crate::classify(name) == crate::PathKind::Local
        && crate::resolve(name, None).is_ok_and(|p| p == Path::new(name));
    if !plain {
        return None;
    }
    let name = match extension {
        Some(e) => Path::new(name).with_extension(e).into_os_string(),
        None => name.into(),
    };
    Some(folder.join(name))
}

/// Relink to Folder: for each of `links`, its [`folder_match`] in `folder` when
/// [`LinkFiles::stat`] finds a regular file there. Returns the matches and the keys of the links
/// that found none, both in `links` order. Reads the disk: call it in a job.
pub fn folder_matches<K: Clone>(links: &[Link<K>], folder: &Path, extension: Option<&str>, files: &dyn LinkFiles) -> (Vec<(K, PathBuf)>, Vec<K>) {
    // designcraft: crates/engine/src/cmd/links.rs:493 (the loop over the links; `notFound`).
    let mut found = Vec::new();
    let mut not_found = Vec::new();
    for link in links {
        match folder_match(link, folder, extension).filter(|p| files.stat(p).is_some()) {
            Some(path) => found.push((link.key.clone(), path)),
            None => not_found.push(link.key.clone()),
        }
    }
    (found, not_found)
}
