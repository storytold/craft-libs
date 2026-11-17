//! Pinned test corpora: a folder of files listed in a manifest, each checked against its SHA-256.
//!
//! A manifest has one tab-separated row per file: the SHA-256 as 64 hexadecimal digits, the
//! file's path relative to the corpus folder, and optional license and URL columns. In this
//! example each `\t` stands for one tab character:
//!
//! ```text
//! # Public Affinity documents
//! cc8ca0db0d8fc545eb29153be900f153fa6b7b7eac068d05e54a502aba1264fe\tafdesignload-color.afdesign\tMIT\thttps://example.org/color.afdesign
//! 9a17f500d09fb3b1a70d24aabdc2882955ec3eac3700a474549e6100be3a8484\tmore/layer_mode.afdesign
//! ```
//!
//! [`Manifest::parse`] skips blank lines and lines whose first character other than a space or a
//! tab is `#`. A `#` later in a row is part of that row. It returns a [`ManifestError`] naming the
//! line of a row with fewer than 2 or more than 4 columns, a digest that is not 64 hexadecimal
//! digits, a path that is empty, absolute or leaves the folder, or a path listed twice. Paths use
//! `/` between their parts.
//!
//! [`pinned`] is the gate a test calls: it returns the corpus folder named by a variable after
//! checking every listed file, prints one `SKIPPED:` line and returns `None` when the variable is
//! unset and the corpus is optional, and fails the test otherwise. [`verify`] runs the same check
//! and returns a [`VerifyError`] instead of failing. No function here downloads a file or changes
//! the process environment.
//!
//! Every input has a size limit, set by [`Limits`]: a manifest of at most 1 MiB and 10,000 rows,
//! and files of at most 8 GiB each, read 64 KiB at a time.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The variable [`pinned`] reads when the caller names none.
pub const DEFAULT_VAR: &str = "CRAFT_CORPUS";

/// The suffix [`pinned`] adds to the folder variable's name to form the name of the variable that
/// makes the corpus required.
pub const REQUIRED_SUFFIX: &str = "_REQUIRED";

/// The size of each read while a file is hashed.
const CHUNK: usize = 64 * 1024;

/// Size limits for a manifest and for the files it lists. An input over a limit is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest manifest in bytes. Default 1 MiB.
    pub max_manifest_bytes: usize,
    /// The most rows a manifest may list. Default 10,000.
    pub max_entries: usize,
    /// The largest listed file in bytes. Default 8 GiB.
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_manifest_bytes: 1024 * 1024, max_entries: 10_000, max_file_bytes: 8 * 1024 * 1024 * 1024 }
    }
}

/// One row of a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    digest: [u8; 32],
    path: String,
    license: Option<String>,
    url: Option<String>,
    line: usize,
}

impl Entry {
    /// The file's SHA-256.
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// The file's path relative to the corpus folder, with `/` between its parts.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The license column, when the row has one that is not empty.
    pub fn license(&self) -> Option<&str> {
        self.license.as_deref()
    }

    /// The URL column, when the row has one that is not empty.
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// The 1-based line of the manifest that holds the row.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The file's path under `dir`.
    fn path_in(&self, dir: &Path) -> PathBuf {
        let mut path = dir.to_path_buf();
        path.extend(self.path.split('/'));
        path
    }
}

/// A parsed manifest: the rows in the order the manifest lists them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    entries: Vec<Entry>,
}

impl Manifest {
    /// Parses `text` under the default [`Limits`].
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        Self::parse_with_limits(text, &Limits::default())
    }

    /// Parses `text`. A manifest longer than `limits.max_manifest_bytes` or with more than
    /// `limits.max_entries` rows is an error.
    pub fn parse_with_limits(text: &str, limits: &Limits) -> Result<Self, ManifestError> {
        if text.len() > limits.max_manifest_bytes {
            return Err(ManifestError::TooLarge { limit: limits.max_manifest_bytes });
        }
        let mut entries: Vec<Entry> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let content = raw.trim_start_matches([' ', '\t']);
            if content.trim().is_empty() || content.starts_with('#') {
                continue;
            }
            if entries.len() == limits.max_entries {
                return Err(ManifestError::TooManyEntries { limit: limits.max_entries });
            }
            let columns: Vec<&str> = raw.split('\t').collect();
            if !(2..=4).contains(&columns.len()) {
                return Err(ManifestError::Columns { line, found: columns.len() });
            }
            let digest = parse_digest(columns[0]).ok_or(ManifestError::Digest { line })?;
            let path = columns[1];
            check_path(path).map_err(|reason| ManifestError::Path { line, reason })?;
            if !seen.insert(path) {
                let first = entries.iter().find(|e| e.path == path).map_or(0, |e| e.line);
                return Err(ManifestError::Duplicate { line, first });
            }
            let optional = |i: usize| columns.get(i).filter(|c| !c.is_empty()).map(|c| (*c).to_owned());
            entries.push(Entry { digest, path: path.to_owned(), license: optional(2), url: optional(3), line });
        }
        Ok(Self { entries })
    }

    /// Reads and parses the manifest file at `path`. A path that is not a regular file is refused
    /// before it is opened. Reading stops after `limits.max_manifest_bytes` bytes plus one, so an
    /// oversized file is not read whole.
    pub fn read(path: &Path, limits: &Limits) -> Result<Self, ManifestError> {
        if !std::fs::metadata(path).map_err(ManifestError::Io)?.is_file() {
            return Err(ManifestError::NotAFile);
        }
        let file = open_for_reading(path).map_err(ManifestError::Io)?;
        if !file.metadata().map_err(ManifestError::Io)?.is_file() {
            return Err(ManifestError::NotAFile);
        }
        let cap = u64::try_from(limits.max_manifest_bytes).unwrap_or(u64::MAX).saturating_add(1);
        let mut bytes = Vec::new();
        file.take(cap).read_to_end(&mut bytes).map_err(ManifestError::Io)?;
        if bytes.len() > limits.max_manifest_bytes {
            return Err(ManifestError::TooLarge { limit: limits.max_manifest_bytes });
        }
        let text = String::from_utf8(bytes).map_err(|_| ManifestError::NotUtf8)?;
        Self::parse_with_limits(&text, limits)
    }

    /// The rows, in manifest order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The paths of the rows, in manifest order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(Entry::path)
    }
}

/// Decodes 64 ASCII hexadecimal digits, in either case.
fn parse_digest(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut digest = [0u8; 32];
    let (pairs, _) = bytes.as_chunks::<2>();
    for (out, &[high, low]) in digest.iter_mut().zip(pairs) {
        let high = char::from(high).to_digit(16)?;
        let low = char::from(low).to_digit(16)?;
        *out = u8::try_from(high * 16 + low).ok()?;
    }
    Some(digest)
}

/// Checks that `path` is relative and stays in the corpus folder: parts separated by `/`, none
/// empty, `.` or `..`, and no `\`, `:` or control character (NUL included) anywhere, so the same
/// row means the same file on every platform.
fn check_path(path: &str) -> Result<(), PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.contains(['\\', ':']) || path.chars().any(char::is_control) {
        return Err(PathError::Character);
    }
    for part in path.split('/') {
        match part {
            "" => return Err(if path.starts_with('/') { PathError::Absolute } else { PathError::EmptyPart }),
            "." | ".." => return Err(PathError::DotPart),
            _ => {}
        }
    }
    Ok(())
}

/// Why a manifest path was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// The path is empty.
    Empty,
    /// The path starts with `/`.
    Absolute,
    /// The path has an empty part (`a//b`, or a trailing `/`).
    EmptyPart,
    /// The path has a `.` or `..` part.
    DotPart,
    /// The path holds a `\`, a `:` or a control character.
    Character,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "the path is empty",
            Self::Absolute => "the path is absolute",
            Self::EmptyPart => "the path has an empty part",
            Self::DotPart => "the path has a . or .. part",
            Self::Character => "the path holds a \\, a : or a control character",
        })
    }
}

/// Why a manifest could not be read or parsed.
#[derive(Debug)]
pub enum ManifestError {
    /// The manifest is longer than the limit in bytes.
    TooLarge {
        /// The limit in bytes.
        limit: usize,
    },
    /// The manifest lists more rows than the limit.
    TooManyEntries {
        /// The limit in rows.
        limit: usize,
    },
    /// A row does not have 2, 3 or 4 tab-separated columns.
    Columns {
        /// The 1-based line of the row.
        line: usize,
        /// The number of columns found.
        found: usize,
    },
    /// A row's digest is not 64 hexadecimal digits.
    Digest {
        /// The 1-based line of the row.
        line: usize,
    },
    /// A row's path is refused.
    Path {
        /// The 1-based line of the row.
        line: usize,
        /// Why the path is refused.
        reason: PathError,
    },
    /// A row repeats the path of an earlier row.
    Duplicate {
        /// The 1-based line of the repeated row.
        line: usize,
        /// The 1-based line of the earlier row.
        first: usize,
    },
    /// The manifest path does not name a regular file.
    NotAFile,
    /// The manifest file is not UTF-8.
    NotUtf8,
    /// The manifest file could not be read.
    Io(io::Error),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { limit } => write!(f, "the manifest is longer than {limit} bytes"),
            Self::TooManyEntries { limit } => write!(f, "the manifest lists more than {limit} files"),
            Self::Columns { line, found } => write!(f, "line {line}: {found} tab-separated columns, expected 2 to 4"),
            Self::Digest { line } => write!(f, "line {line}: the SHA-256 is not 64 hexadecimal digits"),
            Self::Path { line, reason } => write!(f, "line {line}: {reason}"),
            Self::Duplicate { line, first } => write!(f, "line {line}: the path is already listed on line {first}"),
            Self::NotAFile => f.write_str("the manifest is not a regular file"),
            Self::NotUtf8 => f.write_str("the manifest is not UTF-8"),
            Self::Io(error) => write!(f, "the manifest could not be read: {error}"),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Why a corpus folder does not match its manifest.
#[derive(Debug)]
pub enum VerifyError {
    /// The manifest lists no files, so the folder would be accepted without a check.
    EmptyManifest,
    /// The corpus folder could not be resolved.
    Folder {
        /// The folder.
        dir: PathBuf,
        /// The error resolving it.
        error: io::Error,
    },
    /// A listed file is missing or could not be resolved.
    Missing {
        /// The path the manifest lists.
        path: String,
        /// The error resolving it.
        error: io::Error,
    },
    /// A listed path resolves outside the corpus folder, through a symbolic link.
    Escapes {
        /// The path the manifest lists.
        path: String,
    },
    /// A listed path is not a regular file.
    NotAFile {
        /// The path the manifest lists.
        path: String,
    },
    /// A listed file is larger than the limit.
    TooLarge {
        /// The path the manifest lists.
        path: String,
        /// The limit in bytes.
        limit: u64,
    },
    /// A listed file could not be read.
    Read {
        /// The path the manifest lists.
        path: String,
        /// The read error.
        error: io::Error,
    },
    /// A listed file's SHA-256 differs from the manifest's.
    Modified {
        /// The path the manifest lists.
        path: String,
    },
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyManifest => f.write_str("the manifest lists no files"),
            Self::Folder { dir, error } => write!(f, "corpus folder {}: {error}", dir.display()),
            Self::Missing { path, error } => write!(f, "{path}: missing: {error}"),
            Self::Escapes { path } => write!(f, "{path}: resolves outside the corpus folder"),
            Self::NotAFile { path } => write!(f, "{path}: not a regular file"),
            Self::TooLarge { path, limit } => write!(f, "{path}: larger than {limit} bytes"),
            Self::Read { path, error } => write!(f, "{path}: {error}"),
            Self::Modified { path } => write!(f, "{path}: SHA-256 differs from the manifest"),
        }
    }
}

impl std::error::Error for VerifyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Folder { error, .. } | Self::Missing { error, .. } | Self::Read { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Checks every file `manifest` lists in `dir` under the default [`Limits`].
pub fn verify(dir: &Path, manifest: &Manifest) -> Result<(), VerifyError> {
    verify_with_limits(dir, manifest, &Limits::default())
}

/// Checks every file `manifest` lists in `dir`: each must resolve inside `dir` after symbolic
/// links are followed, be a regular file of at most `limits.max_file_bytes` bytes, and have the
/// listed SHA-256. Returns the first failure in manifest order. An empty manifest is an error.
pub fn verify_with_limits(dir: &Path, manifest: &Manifest, limits: &Limits) -> Result<(), VerifyError> {
    if manifest.entries.is_empty() {
        return Err(VerifyError::EmptyManifest);
    }
    let root = dir.canonicalize().map_err(|error| VerifyError::Folder { dir: dir.to_path_buf(), error })?;
    let mut buf = vec![0u8; CHUNK];
    for entry in &manifest.entries {
        verify_entry(&root, entry, limits, &mut buf)?;
    }
    Ok(())
}

fn verify_entry(root: &Path, entry: &Entry, limits: &Limits, buf: &mut [u8]) -> Result<(), VerifyError> {
    verify_entry_with(root, entry, limits, buf, open_for_reading)
}

/// [`verify_entry`] with the function that opens the resolved path; tests change the path in it.
fn verify_entry_with(
    root: &Path,
    entry: &Entry,
    limits: &Limits,
    buf: &mut [u8],
    open: impl FnOnce(&Path) -> io::Result<File>,
) -> Result<(), VerifyError> {
    let path = || entry.path.clone();
    let target = entry.path_in(root).canonicalize().map_err(|error| VerifyError::Missing { path: path(), error })?;
    if !target.starts_with(root) {
        return Err(VerifyError::Escapes { path: path() });
    }
    let kind = std::fs::metadata(&target).map_err(|error| VerifyError::Read { path: path(), error })?;
    if !kind.is_file() {
        return Err(VerifyError::NotAFile { path: path() });
    }
    let file = open(&target).map_err(|error| VerifyError::Read { path: path(), error })?;
    let meta = file.metadata().map_err(|error| VerifyError::Read { path: path(), error })?;
    if !meta.is_file() {
        return Err(VerifyError::NotAFile { path: path() });
    }
    #[cfg(unix)]
    opened_file_is_inside(root, entry, &file, &meta)?;
    if meta.len() > limits.max_file_bytes {
        return Err(VerifyError::TooLarge { path: path(), limit: limits.max_file_bytes });
    }
    let mut reader = file.take(limits.max_file_bytes.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    loop {
        let n = match reader.read(buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(VerifyError::Read { path: path(), error }),
        };
        total = total.saturating_add(n as u64);
        if total > limits.max_file_bytes {
            return Err(VerifyError::TooLarge { path: path(), limit: limits.max_file_bytes });
        }
        hasher.update(&buf[..n]);
    }
    if hasher.finalize().as_slice() != entry.digest.as_slice() {
        return Err(VerifyError::Modified { path: path() });
    }
    Ok(())
}

/// Checks, after the open, that the opened file is inside `root`, so a path changed into a link out
/// of the folder between the first check and the open is refused. On Linux, Android and Apple
/// platforms the kernel reports the open file's path, which must start with `root`. On other Unix
/// systems, and where that path cannot be read, the entry's path is resolved again and must lead, without a link in its last part, to the
/// opened file (the same device and inode).
#[cfg(unix)]
fn opened_file_is_inside(root: &Path, entry: &Entry, file: &File, opened: &std::fs::Metadata) -> Result<(), VerifyError> {
    use std::os::unix::fs::MetadataExt;
    let path = || entry.path.clone();
    if let Some(actual) = opened_path(file).map_err(|error| VerifyError::Read { path: path(), error })? {
        return if actual.starts_with(root) { Ok(()) } else { Err(VerifyError::Escapes { path: path() }) };
    }
    let again = entry.path_in(root).canonicalize().map_err(|error| VerifyError::Missing { path: path(), error })?;
    let now = std::fs::symlink_metadata(&again).map_err(|error| VerifyError::Read { path: path(), error })?;
    if !again.starts_with(root) || (now.dev(), now.ino()) != (opened.dev(), opened.ino()) {
        return Err(VerifyError::Escapes { path: path() });
    }
    Ok(())
}

/// The path of the file `file` has open, as the kernel reports it in `/proc/self/fd`, or `None`
/// when that cannot be read (for example without a mounted `/proc`).
#[cfg(any(target_os = "linux", target_os = "android"))]
#[allow(clippy::unnecessary_wraps)]
fn opened_path(file: &File) -> io::Result<Option<PathBuf>> {
    use std::os::fd::AsRawFd;
    Ok(std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok())
}

/// The path of the file `file` has open, as the kernel reports it.
#[cfg(target_vendor = "apple")]
fn opened_path(file: &File) -> io::Result<Option<PathBuf>> {
    use std::os::unix::ffi::OsStringExt;
    let path = rustix::fs::getpath(file)?;
    Ok(Some(PathBuf::from(OsString::from_vec(path.into_bytes()))))
}

/// `None`: the platform does not report the path of an open file.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))))]
fn opened_path(_file: &File) -> io::Result<Option<PathBuf>> {
    Ok(None)
}

/// Opens `path` for reading. On Unix the file is opened with `O_NONBLOCK`, so a path that was
/// replaced by a named pipe after its type was checked opens at once instead of waiting for a
/// writer; the caller checks the opened file's type. Reads from a regular file are not affected.
fn open_for_reading(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    options.open(path)
}

/// The corpus folder the variable `var` names (default [`DEFAULT_VAR`]), after every file
/// `manifest` lists has been checked with [`verify`].
///
/// The corpus is required when the variable named `var` plus [`REQUIRED_SUFFIX`] (by default
/// `CRAFT_CORPUS_REQUIRED`) is exactly `1`; unset, empty, `0`, `true` and any other value leave it
/// optional.
///
/// - Folder variable unset and the corpus optional: prints one line,
///   `SKIPPED: corpus not given: set <var> to its folder`, to stderr and returns `None`.
/// - Folder variable unset and the corpus required: fails the test.
/// - Folder variable set, even to an empty value: fails the test unless the manifest parses and
///   every listed file matches.
///
/// It fails the test with a panic whose message names the variable and the failure.
#[allow(clippy::panic)]
pub fn pinned(var: Option<&str>, manifest: &str) -> Option<PathBuf> {
    let var = var.unwrap_or(DEFAULT_VAR);
    let required_var = format!("{var}{REQUIRED_SUFFIX}");
    let required = std::env::var_os(&required_var).as_deref() == Some(OsStr::new("1"));
    match gate(std::env::var_os(var), required, var, &required_var, manifest) {
        Ok(Some(dir)) => Some(dir),
        Ok(None) => {
            eprintln!("SKIPPED: corpus not given: set {var} to its folder");
            None
        }
        Err(message) => panic!("{message}"),
    }
}

/// The decision of [`pinned`], without reading the environment or printing: `Ok(None)` skips and
/// `Err` holds the message the test fails with.
fn gate(folder: Option<OsString>, required: bool, var: &str, required_var: &str, manifest: &str) -> Result<Option<PathBuf>, String> {
    let Some(folder) = folder else {
        return if required { Err(format!("corpus not given: {var} is unset and {required_var}=1 forbids skipping")) } else { Ok(None) };
    };
    if folder.is_empty() {
        return Err(format!("{var} is set but empty"));
    }
    let dir = PathBuf::from(folder);
    let manifest = Manifest::parse(manifest).map_err(|e| format!("corpus manifest for {var}: {e}"))?;
    verify(&dir, &manifest).map_err(|e| format!("pinned corpus in {} ({var}): {e}", dir.display()))?;
    Ok(Some(dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Output};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The case the child process runs; unset in an ordinary test run.
    const CHILD_CASE: &str = "CRAFT_TESTKIT_CORPUS_CHILD";
    /// The folder variable the child's `pinned` reads.
    const VAR: &str = "CRAFT_TESTKIT_CORPUS_DIR";
    /// The required variable that goes with [`VAR`].
    const REQUIRED: &str = "CRAFT_TESTKIT_CORPUS_DIR_REQUIRED";

    const HELLO: &[u8] = b"hello world";
    /// SHA-256 of `hello world`.
    const HELLO_SHA: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
    const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// A folder under the system temporary folder, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("craft_testkit_corpus_{}_{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create scratch folder");
            Self(dir)
        }

        fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(&path, bytes).expect("write file");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn parse_err(text: &str) -> ManifestError {
        Manifest::parse(text).expect_err("the manifest is refused")
    }

    #[test]
    fn comments_are_skipped() {
        let text = format!("# pinned corpus\n  # indented comment\n\t# tab-indented comment\n{HELLO_SHA}\thello.txt\n#{EMPTY_SHA}\tempty.txt\n");
        let manifest = Manifest::parse(&text).expect("parse");
        assert_eq!(manifest.names().collect::<Vec<_>>(), ["hello.txt"]);
        assert_eq!(manifest.entries()[0].line(), 4);
    }

    #[test]
    fn a_hash_inside_a_row_is_not_a_comment() {
        let manifest = Manifest::parse(&format!("{HELLO_SHA}\tdir/#1.txt\tMIT\thttps://example.org/a#b\n")).expect("parse");
        let entry = &manifest.entries()[0];
        assert_eq!(entry.path(), "dir/#1.txt");
        assert_eq!(entry.url(), Some("https://example.org/a#b"));
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = format!("\n   \n\t\n{HELLO_SHA}\thello.txt\n\n{EMPTY_SHA}\tempty.txt\n \t \n");
        let manifest = Manifest::parse(&text).expect("parse");
        assert_eq!(manifest.entries().len(), 2);
        assert_eq!(manifest.entries()[0].line(), 4);
        assert_eq!(manifest.entries()[1].line(), 6);
        assert_eq!(Manifest::parse("").expect("parse").entries().len(), 0);
    }

    #[test]
    fn malformed_row_is_error() {
        assert!(matches!(parse_err(HELLO_SHA), ManifestError::Columns { line: 1, found: 1 }));
        assert!(matches!(parse_err(&format!("# c\n{HELLO_SHA}  hello.txt\n")), ManifestError::Columns { line: 2, found: 1 }));
        assert!(matches!(parse_err(&format!("{HELLO_SHA}\ta\tMIT\turl\textra\n")), ManifestError::Columns { line: 1, found: 5 }));
        assert!(matches!(parse_err(&format!("{HELLO_SHA}\t\n")), ManifestError::Path { line: 1, reason: PathError::Empty }));
        assert!(matches!(parse_err("\thello.txt\n"), ManifestError::Digest { line: 1 }));
    }

    #[test]
    fn wrong_digest_length_is_error() {
        assert!(matches!(parse_err(&format!("{}\ta\n", &HELLO_SHA[..63])), ManifestError::Digest { line: 1 }));
        assert!(matches!(parse_err(&format!("{HELLO_SHA}0\ta\n")), ManifestError::Digest { line: 1 }));
        assert!(matches!(parse_err(&format!(" {HELLO_SHA}\ta\n")), ManifestError::Digest { line: 1 }));
    }

    #[test]
    fn non_hex_digest_is_error() {
        let bad = format!("{}g", &HELLO_SHA[..63]);
        assert!(matches!(parse_err(&format!("{bad}\ta\n")), ManifestError::Digest { line: 1 }));
        let sign = format!("+{}", &HELLO_SHA[..63]);
        assert!(matches!(parse_err(&format!("{sign}\ta\n")), ManifestError::Digest { line: 1 }));
        let wide = format!("{}é", &HELLO_SHA[..62]);
        assert!(matches!(parse_err(&format!("{wide}\ta\n")), ManifestError::Digest { line: 1 }));
    }

    #[test]
    fn rows_take_two_to_four_columns_and_empty_optional_columns_are_none() {
        let text = format!(
            "{HELLO_SHA}\ta.txt\n{HELLO_SHA}\tb.txt\tMIT\n{HELLO_SHA}\tc.txt\tMIT\thttps://example.org/c\n{HELLO_SHA}\td.txt\t\n{HELLO_SHA}\te.txt\t\thttps://example.org/e\n"
        );
        let manifest = Manifest::parse(&text).expect("parse");
        let cols: Vec<_> = manifest.entries().iter().map(|e| (e.path(), e.license(), e.url())).collect();
        assert_eq!(
            cols,
            [
                ("a.txt", None, None),
                ("b.txt", Some("MIT"), None),
                ("c.txt", Some("MIT"), Some("https://example.org/c")),
                ("d.txt", None, None),
                ("e.txt", None, Some("https://example.org/e")),
            ]
        );
    }

    #[test]
    fn crlf_lines_and_uppercase_digests_parse() {
        let upper = HELLO_SHA.to_ascii_uppercase();
        let manifest = Manifest::parse(&format!("# c\r\n{upper}\thello.txt\tMIT\r\n\r\n")).expect("parse");
        let entry = &manifest.entries()[0];
        assert_eq!(entry.license(), Some("MIT"));
        assert_eq!(entry.digest()[..4], [0xb9, 0x4d, 0x27, 0xb9]);
        assert_eq!(entry.digest(), Manifest::parse(&format!("{HELLO_SHA}\thello.txt")).expect("parse").entries()[0].digest());
    }

    #[test]
    fn duplicate_path_is_error() {
        let text = format!("{HELLO_SHA}\ta/b.txt\n# c\n{HELLO_SHA}\ta/b.txt\n");
        assert!(matches!(parse_err(&text), ManifestError::Duplicate { line: 3, first: 1 }));
    }

    #[test]
    fn paths_that_leave_the_folder_are_errors() {
        let cases = [
            ("/etc/passwd", PathError::Absolute),
            ("../outside", PathError::DotPart),
            ("a/../../outside", PathError::DotPart),
            ("./a", PathError::DotPart),
            ("a//b", PathError::EmptyPart),
            ("a/", PathError::EmptyPart),
            ("C:/Windows/win.ini", PathError::Character),
            ("C:a", PathError::Character),
            ("\\\\server\\share\\a", PathError::Character),
            ("a\\..\\..\\b", PathError::Character),
            ("a\u{1}b", PathError::Character),
            ("a\u{7f}b", PathError::Character),
            ("a\u{85}b", PathError::Character),
        ];
        for (path, expected) in cases {
            match parse_err(&format!("{HELLO_SHA}\t{path}\n")) {
                ManifestError::Path { line: 1, reason } => assert_eq!(reason, expected, "{path}"),
                other => panic!("{path}: {other}"),
            }
        }
    }

    #[test]
    fn manifest_limits_are_errors() {
        let text = format!("{HELLO_SHA}\ta\n{HELLO_SHA}\tb\n");
        let limits = Limits { max_entries: 1, ..Limits::default() };
        assert!(matches!(Manifest::parse_with_limits(&text, &limits), Err(ManifestError::TooManyEntries { limit: 1 })));
        let limits = Limits { max_entries: 2, ..Limits::default() };
        assert_eq!(Manifest::parse_with_limits(&text, &limits).expect("parse").entries().len(), 2);

        let limits = Limits { max_manifest_bytes: text.len() - 1, ..Limits::default() };
        assert!(matches!(Manifest::parse_with_limits(&text, &limits), Err(ManifestError::TooLarge { .. })));
        let limits = Limits { max_manifest_bytes: text.len(), ..Limits::default() };
        assert!(Manifest::parse_with_limits(&text, &limits).is_ok());

        let scratch = Scratch::new();
        let path = scratch.write("corpus.tsv", text.as_bytes());
        assert_eq!(Manifest::read(&path, &Limits::default()).expect("read").entries().len(), 2);
        let limits = Limits { max_manifest_bytes: text.len() - 1, ..Limits::default() };
        assert!(matches!(Manifest::read(&path, &limits), Err(ManifestError::TooLarge { .. })));
        let bad = scratch.write("bad.tsv", b"\xff\xfe");
        assert!(matches!(Manifest::read(&bad, &Limits::default()), Err(ManifestError::NotUtf8)));
        assert!(matches!(Manifest::read(&scratch.0.join("absent.tsv"), &Limits::default()), Err(ManifestError::Io(_))));
    }

    fn hello_corpus() -> (Scratch, Manifest) {
        let scratch = Scratch::new();
        scratch.write("hello.txt", HELLO);
        scratch.write("sub/empty.bin", b"");
        let manifest = Manifest::parse(&format!("{HELLO_SHA}\thello.txt\tMIT\n{EMPTY_SHA}\tsub/empty.bin\n")).expect("parse");
        (scratch, manifest)
    }

    #[test]
    fn a_matching_folder_verifies() {
        let (scratch, manifest) = hello_corpus();
        verify(&scratch.0, &manifest).expect("the corpus matches");
    }

    #[test]
    fn missing_file_fails() {
        let (scratch, manifest) = hello_corpus();
        std::fs::remove_file(scratch.0.join("sub").join("empty.bin")).expect("remove");
        match verify(&scratch.0, &manifest) {
            Err(VerifyError::Missing { path, .. }) => assert_eq!(path, "sub/empty.bin"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(verify(&scratch.0.join("absent"), &manifest), Err(VerifyError::Folder { .. })));
    }

    #[test]
    fn modified_file_fails() {
        let (scratch, manifest) = hello_corpus();
        scratch.write("hello.txt", b"hello world!");
        match verify(&scratch.0, &manifest) {
            Err(VerifyError::Modified { path }) => assert_eq!(path, "hello.txt"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_empty_manifest_or_a_folder_entry_fails() {
        let (scratch, _) = hello_corpus();
        assert!(matches!(verify(&scratch.0, &Manifest::default()), Err(VerifyError::EmptyManifest)));
        let manifest = Manifest::parse(&format!("{EMPTY_SHA}\tsub\n")).expect("parse");
        assert!(matches!(verify(&scratch.0, &manifest), Err(VerifyError::NotAFile { .. })));
    }

    /// A named pipe is refused before it is opened; opening one for reading waits for a writer.
    #[cfg(unix)]
    #[test]
    fn a_named_pipe_fails_without_being_opened() {
        let scratch = Scratch::new();
        let fifo = scratch.0.join("pipe");
        let made = Command::new("mkfifo").arg(&fifo).status().expect("run mkfifo");
        assert!(made.success(), "mkfifo failed");
        let manifest = Manifest::parse(&format!("{EMPTY_SHA}\tpipe\n")).expect("parse");
        match verify(&scratch.0, &manifest) {
            Err(VerifyError::NotAFile { path }) => assert_eq!(path, "pipe"),
            other => panic!("{other:?}"),
        }
    }

    /// A manifest path that names a named pipe or a folder is refused before it is opened.
    #[cfg(unix)]
    #[test]
    fn a_manifest_that_is_not_a_file_fails_without_being_opened() {
        let scratch = Scratch::new();
        let fifo = scratch.0.join("corpus.tsv");
        let made = Command::new("mkfifo").arg(&fifo).status().expect("run mkfifo");
        assert!(made.success(), "mkfifo failed");
        assert!(matches!(Manifest::read(&scratch.0, &Limits::default()), Err(ManifestError::NotAFile)));
        assert!(matches!(Manifest::read(&fifo, &Limits::default()), Err(ManifestError::NotAFile)));
    }

    /// Files are opened with `O_NONBLOCK`, read back from the open file's status flags in
    /// `/proc/self/fdinfo`, so a path replaced by a named pipe after its type was checked opens at
    /// once instead of waiting for a writer.
    #[cfg(target_os = "linux")]
    #[test]
    fn files_are_opened_with_o_nonblock() {
        use std::os::fd::AsRawFd;
        let (scratch, _) = hello_corpus();
        let file = open_for_reading(&scratch.0.join("hello.txt")).expect("open");
        let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", file.as_raw_fd())).expect("read fdinfo");
        let flags = info.lines().find_map(|line| line.strip_prefix("flags:")).map(str::trim).expect("flags line");
        let flags = i32::from_str_radix(flags, 8).expect("octal flags");
        assert_ne!(flags & libc::O_NONBLOCK, 0, "flags {flags:o}");
    }

    #[test]
    fn file_size_limit_is_error() {
        let (scratch, manifest) = hello_corpus();
        let limits = Limits { max_file_bytes: HELLO.len() as u64, ..Limits::default() };
        verify_with_limits(&scratch.0, &manifest, &limits).expect("a file at the limit passes");
        let limits = Limits { max_file_bytes: HELLO.len() as u64 - 1, ..Limits::default() };
        match verify_with_limits(&scratch.0, &manifest, &limits) {
            Err(VerifyError::TooLarge { path, limit }) => assert_eq!((path.as_str(), limit), ("hello.txt", 10)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_file_larger_than_one_read_hashes_whole() {
        let scratch = Scratch::new();
        let bytes: Vec<u8> = (0..CHUNK * 2 + 7).map(|i| (i % 251) as u8).collect();
        scratch.write("big.bin", &bytes);
        let hex: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        let manifest = Manifest::parse(&format!("{hex}\tbig.bin\n")).expect("parse");
        verify(&scratch.0, &manifest).expect("the whole file is hashed");
        scratch.write("big.bin", &bytes[..CHUNK]);
        assert!(matches!(verify(&scratch.0, &manifest), Err(VerifyError::Modified { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_out_of_the_folder_fails() {
        let outside = Scratch::new();
        let target = outside.write("hello.txt", HELLO);
        let corpus = Scratch::new();
        std::os::unix::fs::symlink(&target, corpus.0.join("hello.txt")).expect("symlink");
        let manifest = Manifest::parse(&format!("{HELLO_SHA}\thello.txt\n")).expect("parse");
        match verify(&corpus.0, &manifest) {
            Err(VerifyError::Escapes { path }) => assert_eq!(path, "hello.txt"),
            other => panic!("{other:?}"),
        }
        std::os::unix::fs::symlink(&outside.0, corpus.0.join("out")).expect("symlink");
        let manifest = Manifest::parse(&format!("{HELLO_SHA}\tout/hello.txt\n")).expect("parse");
        assert!(matches!(verify(&corpus.0, &manifest), Err(VerifyError::Escapes { .. })));
    }

    /// A file opened outside the folder, as after the listed path was changed into a link between
    /// the first check and the open, is refused even when its contents match.
    #[cfg(unix)]
    #[test]
    fn a_file_opened_outside_the_folder_fails() {
        let (scratch, manifest) = hello_corpus();
        let outside = Scratch::new();
        let other = outside.write("hello.txt", HELLO);
        let root = scratch.0.canonicalize().expect("canonical folder");
        let entry = &manifest.entries[0];
        let opened = File::open(&other).expect("open the outside file");
        let meta = opened.metadata().expect("metadata");
        match opened_file_is_inside(&root, entry, &opened, &meta) {
            Err(VerifyError::Escapes { path }) => assert_eq!(path, "hello.txt"),
            other => panic!("{other:?}"),
        }
        let listed = File::open(root.join("hello.txt")).expect("open the listed file");
        let meta = listed.metadata().expect("metadata");
        opened_file_is_inside(&root, entry, &listed, &meta).expect("the listed file passes");
    }

    /// `verify` refuses a listed file that is changed into a link out of the folder after it was
    /// resolved and before it is opened, even when the outside file's contents match.
    #[cfg(unix)]
    #[test]
    fn a_file_changed_into_an_outside_link_before_the_open_fails() {
        let (scratch, manifest) = hello_corpus();
        let outside = Scratch::new();
        let other = outside.write("hello.txt", HELLO);
        let root = scratch.0.canonicalize().expect("canonical folder");
        let mut buf = vec![0u8; CHUNK];
        let swap = |path: &Path| {
            std::fs::remove_file(path)?;
            std::os::unix::fs::symlink(&other, path)?;
            open_for_reading(path)
        };
        match verify_entry_with(&root, &manifest.entries[0], &Limits::default(), &mut buf, swap) {
            Err(VerifyError::Escapes { path }) => assert_eq!(path, "hello.txt"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_gate_skips_only_an_optional_unset_corpus() {
        let (scratch, _) = hello_corpus();
        let manifest = format!("{HELLO_SHA}\thello.txt\n");
        assert_eq!(gate(None, false, VAR, REQUIRED, &manifest), Ok(None));
        let required = gate(None, true, VAR, REQUIRED, &manifest).expect_err("required");
        assert!(required.contains(&format!("{REQUIRED}=1")), "{required}");
        let dir = scratch.0.clone().into_os_string();
        assert_eq!(gate(Some(dir.clone()), false, VAR, REQUIRED, &manifest), Ok(Some(scratch.0.clone())));
        let empty = gate(Some(OsString::new()), false, VAR, REQUIRED, &manifest).expect_err("empty");
        assert!(empty.contains("empty"), "{empty}");
        let bad = gate(Some(dir.clone()), false, VAR, REQUIRED, "# c\nnot a row\n").expect_err("bad manifest");
        assert!(bad.contains("line 2"), "{bad}");
        let missing = gate(Some(dir), false, VAR, REQUIRED, &format!("{EMPTY_SHA}\tabsent.bin\n")).expect_err("missing");
        assert!(missing.contains("absent.bin"), "{missing}");
    }

    /// Runs `child` below in a new process of this test binary with the given variables set and
    /// every other corpus variable removed, so no test changes this process's environment.
    fn run_child(case: &str, vars: &[(&str, &str)]) -> Output {
        let exe = std::env::current_exe().expect("the test binary's path");
        let mut cmd = Command::new(exe);
        cmd.args(["corpus::tests::child", "--exact", "--nocapture", "--test-threads=1"]).env(CHILD_CASE, case);
        for name in [VAR, REQUIRED, DEFAULT_VAR, "CRAFT_CORPUS_REQUIRED"] {
            cmd.env_remove(name);
        }
        for (name, value) in vars {
            cmd.env(name, value);
        }
        cmd.output().expect("run the test binary")
    }

    #[test]
    fn child() {
        let Some(case) = std::env::var_os(CHILD_CASE) else {
            return;
        };
        let manifest = format!("{HELLO_SHA}\thello.txt\n");
        let found = match case.to_str() {
            Some("named") => pinned(Some(VAR), &manifest),
            Some("default") => pinned(None, &manifest),
            other => panic!("unknown child case {other:?}"),
        };
        eprintln!("pinned returned {}", found.map_or_else(|| "none".to_owned(), |d| d.display().to_string()));
    }

    #[test]
    fn required_variable_unset_fails() {
        for value in [None, Some(""), Some("0"), Some("true"), Some(" 1"), Some("1\n")] {
            let vars: Vec<(&str, &str)> = value.map(|v| (REQUIRED, v)).into_iter().collect();
            let out = run_child("named", &vars);
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "{value:?}: {err}");
            assert_eq!(err.matches(&format!("SKIPPED: corpus not given: set {VAR} to its folder\n")).count(), 1, "{value:?}: {err}");
            assert!(err.contains("pinned returned none"), "{value:?}: {err}");
        }
        let out = run_child("named", &[(REQUIRED, "1")]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{err}");
        assert!(err.contains(&format!("{REQUIRED}=1 forbids skipping")), "{err}");
        assert!(!err.contains("SKIPPED:"), "{err}");
        assert!(!err.contains("pinned returned"), "{err}");
    }

    #[test]
    fn pinned_reads_the_named_folder_and_fails_on_a_bad_one() {
        let (scratch, _) = hello_corpus();
        let dir = scratch.0.to_str().expect("a UTF-8 temporary path");
        let out = run_child("named", &[(VAR, dir)]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{err}");
        assert!(err.contains(&format!("pinned returned {dir}")), "{err}");

        scratch.write("hello.txt", b"changed");
        let out = run_child("named", &[(VAR, dir)]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{err}");
        assert!(err.contains("hello.txt: SHA-256 differs from the manifest"), "{err}");

        let out = run_child("named", &[(VAR, "")]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{err}");
        assert!(err.contains(&format!("{VAR} is set but empty")), "{err}");
    }

    #[test]
    fn pinned_defaults_to_craft_corpus() {
        let out = run_child("default", &[]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{err}");
        assert!(err.contains("SKIPPED: corpus not given: set CRAFT_CORPUS to its folder\n"), "{err}");

        let out = run_child("default", &[("CRAFT_CORPUS_REQUIRED", "1")]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{err}");
        assert!(err.contains("CRAFT_CORPUS_REQUIRED=1 forbids skipping"), "{err}");
    }
}
