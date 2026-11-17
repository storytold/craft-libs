//! The pinned PngSuite. `pngsuite.tsv` lists every file of the suite as a `sha256<TAB>name` row,
//! with `#` comment lines; further tab-separated columns are allowed and ignored. The corpus tests
//! read only the listed PNGs from `corpus/pngsuite/` at the repo root, and only after each file's
//! SHA-256 matches its row. A file the list does not name is never read.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The pinned list.
pub const MANIFEST: &str = include_str!("pngsuite.tsv");

/// The largest file read from the corpus folder. The suite's largest file is under 5 KiB.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// `corpus/pngsuite` at the repo root.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/pngsuite")
}

/// The rows of `manifest` as name to lowercase hex SHA-256. A row needs a 64-digit lowercase hex
/// digest and a plain file name (ASCII letters, digits, `.`, `_` and `-`, not starting with `.`),
/// and each name may appear once.
pub fn parse(manifest: &str) -> Result<BTreeMap<&str, &str>, String> {
    let mut rows = BTreeMap::new();
    for (number, line) in manifest.lines().enumerate().map(|(i, l)| (i + 1, l)) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(digest), Some(name)) = (cols.next(), cols.next()) else {
            return Err(format!("line {number}: expected a SHA-256, a tab and a name"));
        };
        if digest.len() != 64 || !digest.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(format!("line {number}: invalid SHA-256 for {name:?}"));
        }
        if name.is_empty() || name.starts_with('.') || !name.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)) {
            return Err(format!("line {number}: invalid file name {name:?}"));
        }
        if rows.insert(name, digest).is_some() {
            return Err(format!("line {number}: {name} is listed twice"));
        }
    }
    if rows.is_empty() {
        return Err("the manifest lists no files".into());
    }
    Ok(rows)
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Reads a regular file of at most `limit` bytes. A symbolic link, a folder or a larger file is an
/// error, and a file that grows during the read is cut off at `limit + 1` bytes and refused.
fn read_file(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > limit {
        return Err(format!("{}: expected a regular file of at most {limit} bytes", path.display()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(limit + 1).read_to_end(&mut bytes)).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{}: exceeds {limit} bytes", path.display()));
    }
    Ok(bytes)
}

/// Every `.png` that `manifest` lists, read from `dir` in name order with its bytes. A listed PNG
/// that is missing, is not a regular file or does not match its SHA-256 is an error.
pub fn pinned_pngs(dir: &Path, manifest: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let rows = parse(manifest)?;
    let mut out = Vec::new();
    for (name, digest) in rows.into_iter().filter(|(name, _)| name.ends_with(".png")) {
        let bytes = read_file(&dir.join(name), MAX_FILE_BYTES)?;
        let got = sha256_hex(&bytes);
        if got != digest {
            return Err(format!("{name}: SHA-256 {got} does not match the pinned {digest}"));
        }
        out.push((name.to_string(), bytes));
    }
    Ok(out)
}

/// The pinned PNGs of `corpus/pngsuite/`. A missing or changed file fails the calling test.
pub fn pngs() -> Vec<(String, Vec<u8>)> {
    let dir = dir();
    pinned_pngs(&dir, MANIFEST).unwrap_or_else(|e| {
        panic!("{}: {e}; extract the archive named in tests/it/common/pngsuite.tsv into this folder (see the crate README)", dir.display())
    })
}
