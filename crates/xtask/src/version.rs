//! `cargo xtask version [set X.Y.Z[-pre]]`: the workspace version in the root `Cargo.toml`.
//!
//! The version is the `version` key of `[workspace.package]`; member crates inherit it with
//! `version.workspace = true`. [`read`] returns it, [`validate`] checks a new one and [`replace`]
//! returns the manifest with the new version, together with the `version` of every path dependency
//! in `[workspace.dependencies]` whose requirement is the old version (`"OLD"` or `"=OLD"`).
//! [`run`] is the command: the caller passes its workspace root, the Cargo program to run and the
//! words after `version`.
//!
//! The manifest is read with a small TOML scanner that keeps every byte outside the changed strings,
//! comments and line endings included. The root version may be written in any TOML form
//! (`[workspace.package]` with `version = "…"`, a dotted key, an inline table), with double or
//! single quotes. A manifest the scanner cannot read, a duplicate key, a version that is not a
//! one-line string, or a current version that is not valid returns an error before anything is
//! written.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The largest root `Cargo.toml` [`run`] reads, in bytes.
pub const DEFAULT_MAX_MANIFEST_BYTES: u64 = 1 << 20;
/// The largest `Cargo.lock` [`run`] keeps in memory to put back after a failed `cargo update`, in bytes.
pub const DEFAULT_MAX_LOCK_BYTES: u64 = 64 << 20;
/// How deeply arrays and inline tables may nest in the manifest.
const MAX_DEPTH: usize = 64;
const USAGE: &str = "usage: cargo xtask version [set X.Y.Z[-pre]]";
const ROOT_VERSION: [&str; 3] = ["workspace", "package", "version"];

/// Size limits for [`run_with`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The largest root `Cargo.toml` that is read; a larger one is an error before anything is written.
    pub max_manifest_bytes: u64,
    /// The largest `Cargo.lock` that is kept to put back after a failed update; a larger one is an
    /// error before anything is written.
    pub max_lock_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_manifest_bytes: DEFAULT_MAX_MANIFEST_BYTES, max_lock_bytes: DEFAULT_MAX_LOCK_BYTES }
    }
}

impl Limits {
    /// Sets the largest root `Cargo.toml` that is read.
    pub fn with_max_manifest_bytes(mut self, bytes: u64) -> Self {
        self.max_manifest_bytes = bytes;
        self
    }

    /// Sets the largest `Cargo.lock` that is kept for a restore.
    pub fn with_max_lock_bytes(mut self, bytes: u64) -> Self {
        self.max_lock_bytes = bytes;
        self
    }
}

/// The `version` of `[workspace.package]`.
pub fn read(manifest: &str) -> Result<String, String> {
    let entries = scan(manifest)?;
    Ok(root_version(&entries)?.text.clone())
}

/// `manifest` with the workspace version set to `new`, and every path dependency in
/// `[workspace.dependencies]` whose `version` is the old version (`"OLD"` or `"=OLD"`) moved with
/// it. Registry dependencies and path dependencies at other versions keep their requirement, and
/// every other byte is unchanged.
pub fn replace(manifest: &str, new: &str) -> Result<String, String> {
    validate(new)?;
    let entries = scan(manifest)?;
    let root = root_version(&entries)?;
    let old = root.text.as_str();
    validate(old).map_err(|e| format!("the current [workspace.package] version is not valid: {e}"))?;
    let mut edits = vec![(root, new.to_owned())];
    edits.extend(path_pins(&entries, old).into_iter().map(|(pin, prefix)| (pin, format!("{prefix}{new}"))));
    edits.sort_by_key(|(s, _)| s.span.start);
    let mut out = String::with_capacity(manifest.len() + edits.len() * new.len());
    let mut at = 0;
    for (s, text) in edits {
        out.push_str(slice(manifest, at..s.span.start)?);
        let q = char::from(s.quote);
        out.push(q);
        out.push_str(&text);
        out.push(q);
        at = s.span.end;
    }
    out.push_str(slice(manifest, at..manifest.len())?);
    Ok(out)
}

/// Semver without build metadata: `MAJOR.MINOR.PATCH` plus an optional `-pre.release` tag.
///
/// The three numbers have no leading zero and fit in a `u64`. Each prerelease identifier is
/// nonempty ASCII letters, digits and `-`, and a numeric one has no leading zero.
pub fn validate(v: &str) -> Result<(), String> {
    let bad = || Err(format!("`{v}` is not a version like 1.2.3 or 1.2.3-rc.1"));
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums: Vec<&str> = core.split('.').collect();
    if nums.len() != 3
        || nums.iter().any(|n| !n.bytes().all(|b| b.is_ascii_digit()) || (n.len() > 1 && n.starts_with('0')) || n.parse::<u64>().is_err())
    {
        return bad();
    }
    let bad_id = |id: &str| {
        id.is_empty()
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || (id.len() > 1 && id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit()))
    };
    if let Some(p) = pre
        && (p.is_empty() || p.split('.').any(bad_id))
    {
        return bad();
    }
    Ok(())
}

/// `cargo xtask version [set X.Y.Z[-pre]]` with the default [`Limits`].
///
/// `root` is the workspace root, `cargo_program` the Cargo to run there (an app usually passes its
/// `CARGO` variable, or `cargo`), and `args` the words after `version`.
///
/// - With no words, prints the version.
/// - With `set NEW`, writes the new version into `Cargo.toml` (see [`replace`]), runs
///   `cargo update --workspace --offline` and, when that fails, `cargo update --workspace`, then prints
///   `version: OLD -> NEW`. When both updates fail, `Cargo.toml` and `Cargo.lock` are put back as they
///   were and the error says so. Setting the current version prints `version: OLD (unchanged)` and
///   writes only its lock file. One `set` runs at a time in a workspace: it holds its locks (see
///   [`lock_dirs`]) from reading `Cargo.toml` until it returns, and a `set` that finds a lock held
///   returns an error naming the other run and writes nothing.
/// - Other words return the usage error.
pub fn run(root: &Path, cargo_program: &OsStr, args: &[&str]) -> Result<(), String> {
    run_with(root, cargo_program, args, &Limits::default())
}

/// [`run`] with the caller's [`Limits`].
pub fn run_with(root: &Path, cargo_program: &OsStr, args: &[&str], limits: &Limits) -> Result<(), String> {
    run_to(root, args, limits, None, &mut io::stdout(), || update_lock(root, cargo_program))
}

/// The directories of the owner-labeled locks that a `set` in the workspace at `root` holds.
///
/// Before these, a `set` locks the workspace directory itself on Unix, and on Windows
/// `workspace.lock` in the first directory, opened so that it cannot be deleted while held. Every
/// process that writes the workspace shares that lock, and `cargo clean` cannot remove it.
///
/// The first directory is `target/xtask-version/version-<hash of the canonical root>` in the
/// workspace: its lock is required and its owner file names the run that holds it. The second, when
/// there is a home directory, is `~/.cache/craft-xtask/version-<hash of the canonical root>`; a
/// `set` also takes the lock there when it can open the lock file.
///
/// The locks are not on `Cargo.toml` or `Cargo.lock` because a lock on Windows is mandatory and
/// would stop the replacement of `Cargo.toml` and Cargo's own use of `Cargo.lock`.
pub fn lock_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let canonical = std::fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let name = format!("version-{:016x}", fnv1a(canonical.to_string_lossy().as_bytes()));
    // A `target` linked from several workspaces holds one directory per workspace.
    let mut dirs = vec![canonical.join("target").join("xtask-version").join(&name)];
    dirs.extend(std::env::home_dir().filter(|h| h.is_absolute()).map(|h| h.join(".cache").join("craft-xtask").join(name)));
    Ok(dirs)
}

/// The 64-bit FNV-1a hash, which stays the same across Rust releases.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// [`run_with`] that prints to `out`, refreshes `Cargo.lock` with `update`, and keeps its locks in
/// `lock_at` (or [`lock_dirs`] when `None`).
fn run_to(
    root: &Path,
    args: &[&str],
    limits: &Limits,
    lock_at: Option<&[PathBuf]>,
    out: &mut dyn Write,
    update: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let new = match args {
        [] => None,
        ["set", new] => Some(*new),
        _ => return Err(USAGE.into()),
    };
    let manifest = root.join("Cargo.toml");
    let load = || -> Result<(String, String), String> {
        let text =
            String::from_utf8(read_bounded(&manifest, limits.max_manifest_bytes)?).map_err(|_| format!("{}: not UTF-8", manifest.display()))?;
        let old = read(&text)?;
        Ok((text, old))
    };
    let Some(new) = new else {
        return say(out, &load()?.1);
    };
    validate(new)?;
    // Held from the read of `Cargo.toml` through any restore, so that two `set` runs never interleave
    // and an unchanged version is not reported while another run may still put an older one back.
    let _held = lock_workspace(root, lock_at, new)?;
    let (text, old) = load()?;
    let updated = replace(&text, new)?;
    if old == new {
        return say(out, &format!("version: {old} (unchanged)"));
    }
    let lock_path = root.join("Cargo.lock");
    let lock = match std::fs::symlink_metadata(&lock_path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        _ => Some(read_bounded(&lock_path, limits.max_lock_bytes)?),
    };
    write_file(&manifest, updated.as_bytes())?;
    if let Err(e) = update() {
        return Err(restore(&manifest, &text, &old, &lock_path, lock.as_deref(), e));
    }
    say(out, &format!("version: {old} -> {new}"))
}

/// The locks a `set` holds until it returns.
struct WorkspaceLock {
    _base: File,
    _held: Vec<crate::lock::Held>,
}

/// The lock that every process writing the workspace at `root` shares and that `cargo clean` cannot
/// remove while it is held, or `None` when another process holds it. On Unix it is a lock on the
/// workspace directory itself. On Windows it is `workspace.lock` in `dir`, opened without delete
/// sharing, so the file cannot be deleted while it is open.
fn base_lock(root: &Path, dir: &Path) -> Result<Option<File>, String> {
    let fail = |e: io::Error| format!("could not take the version lock, nothing was written: {e}");
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;
        let _ = root;
        std::fs::create_dir_all(dir).map_err(fail)?;
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(dir.join("workspace.lock"))
            .map_err(fail)?
    };
    #[cfg(not(windows))]
    let file = {
        let _ = dir;
        File::open(root).map_err(fail)?
    };
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(fail(e)),
    }
}

/// Takes the workspace's `version set` locks without waiting: [`base_lock`], then the owner-labeled
/// locks in `lock_at` (or [`lock_dirs`]). The lock in the first directory is required; a later
/// directory whose lock file cannot be opened is skipped. When another run holds any of them,
/// releases the others and returns an error that names that run.
fn lock_workspace(root: &Path, lock_at: Option<&[PathBuf]>, new: &str) -> Result<WorkspaceLock, String> {
    let dirs = match lock_at {
        Some(dirs) => dirs.to_vec(),
        None => lock_dirs(root)?,
    };
    let label = format!("pid {} in {}: cargo xtask version set {new}", std::process::id(), root.display());
    let busy = |dir: &Path| {
        let config = crate::lock::LockConfig::new(dir, 1, "");
        let other = crate::lock::holders(&config).ok().and_then(|h| h.into_iter().next()).unwrap_or_else(|| "another run".to_owned());
        format!("another `cargo xtask version set` is running in this workspace ({other}); nothing was written, try again when it ends")
    };
    let first = dirs.first().ok_or_else(|| "could not take the version lock: no lock directory".to_owned())?;
    let Some(base) = base_lock(root, first)? else {
        return Err(busy(first));
    };
    let mut held = Vec::new();
    for (i, dir) in dirs.iter().enumerate() {
        let config = crate::lock::LockConfig::new(dir, 1, label.clone());
        match crate::lock::try_acquire(&config) {
            Ok(Some(h)) => held.push(h),
            Ok(None) => return Err(busy(dir)),
            Err(e) if i == 0 => return Err(format!("could not take the version lock, nothing was written: {e}")),
            Err(_) => {}
        }
    }
    Ok(WorkspaceLock { _base: base, _held: held })
}

fn say(out: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(out, "{line}").map_err(|e| format!("write the output: {e}"))
}

/// Puts `Cargo.toml` and `Cargo.lock` back after `cargo update` failed with `error`.
fn restore(manifest: &Path, text: &str, old: &str, lock_path: &Path, lock: Option<&[u8]>, error: String) -> String {
    let mut msg = match write_file(manifest, text.as_bytes()) {
        Ok(()) => format!("{error}; Cargo.toml restored to {old}"),
        Err(r) => format!("{error}; could not restore Cargo.toml: {r}"),
    };
    let lock_back = match lock {
        Some(bytes) => write_file(lock_path, bytes),
        None => match std::fs::remove_file(lock_path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(format!("remove {}: {e}", lock_path.display())),
            _ => Ok(()),
        },
    };
    if let Err(r) = lock_back {
        msg.push_str(&format!("; could not restore Cargo.lock: {r}"));
    }
    msg
}

/// The file at `path`, or an error when it is larger than `max` bytes.
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let too_big = |len: u64| format!("{}: {len} bytes, more than the {max}-byte limit", path.display());
    let file = File::open(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let len = file.metadata().map_err(|e| format!("read {}: {e}", path.display()))?.len();
    if len > max {
        return Err(too_big(len));
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() as u64 > max {
        return Err(too_big(bytes.len() as u64));
    }
    Ok(bytes)
}

/// Replaces `path` atomically: a half-written root manifest breaks every build in the workspace.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    craft_atomic::atomic_write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Refreshes the workspace members' entries in `Cargo.lock`, without upgrading dependencies.
fn update_lock(root: &Path, cargo_program: &OsStr) -> Result<(), String> {
    let update = |offline: bool| {
        let mut c = Command::new(cargo_program);
        c.current_dir(root).args(["update", "--workspace"]);
        if offline {
            c.arg("--offline");
        }
        crate::runner::run(c, if offline { "cargo update --workspace --offline" } else { "cargo update --workspace" })
    };
    update(true).or_else(|_| update(false))
}

fn slice(s: &str, r: Range<usize>) -> Result<&str, String> {
    s.get(r).ok_or_else(|| "Cargo.toml: an edit does not fall on a character boundary".to_owned())
}

/// A one-line string value: the byte range of its quoted token, its text and its quote character.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Str {
    span: Range<usize>,
    text: String,
    quote: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Str(Str),
    /// A multi-line string.
    LongStr,
    /// A number, boolean, date, array or inline table.
    Other,
}

/// A key of the manifest with its full dotted path (table, dotted key and inline tables).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: Vec<String>,
    value: Value,
}

/// The `[workspace.package]` version string.
fn root_version(entries: &[Entry]) -> Result<&Str, String> {
    let mut found = entries.iter().filter(|e| e.path.iter().map(String::as_str).eq(ROOT_VERSION));
    let first = found.next().ok_or("no `version` in [workspace.package] of the root Cargo.toml")?;
    if found.next().is_some() {
        return Err("[workspace.package] version is set more than once in Cargo.toml".into());
    }
    match &first.value {
        Value::Str(s) if s.text.is_empty() => Err("empty [workspace.package] version".into()),
        Value::Str(s) => Ok(s),
        Value::LongStr => Err("[workspace.package] version is a multi-line string; write it as `version = \"X.Y.Z\"`".into()),
        Value::Other => Err("[workspace.package] version is not a string".into()),
    }
}

/// The `version` strings of path dependencies in `[workspace.dependencies]` that require `old`, each
/// with the operator in front of the version (`""` or `"="`).
fn path_pins<'a>(entries: &'a [Entry], old: &str) -> Vec<(&'a Str, String)> {
    let dep = |e: &'a Entry| match e.path.as_slice() {
        [w, d, name, key] if w == "workspace" && d == "dependencies" => Some((name.as_str(), key.as_str(), &e.value)),
        _ => None,
    };
    let with_path: HashSet<&str> = entries.iter().filter_map(dep).filter(|(_, k, _)| *k == "path").map(|(n, _, _)| n).collect();
    entries
        .iter()
        .filter_map(dep)
        .filter_map(|(name, key, value)| match value {
            Value::Str(s) if key == "version" => Some((name, s)),
            _ => None,
        })
        .filter_map(|(name, s)| {
            let prefix = s.text.strip_suffix(old)?;
            matches!(prefix.trim(), "" | "=").then(|| (name, s, prefix.to_owned()))
        })
        .filter(|(name, _, _)| with_path.contains(name))
        .map(|(_, s, prefix)| (s, prefix))
        .collect()
}

/// Every key of `manifest` whose value is not inside an array.
fn scan(manifest: &str) -> Result<Vec<Entry>, String> {
    let mut s = Scanner { s: manifest, b: manifest.as_bytes(), i: 0, table: Vec::new(), out: Vec::new() };
    loop {
        s.skip_blank()?;
        match s.peek() {
            None => return Ok(s.out),
            Some(b'[') => s.header()?,
            Some(_) => s.key_value()?,
        }
        s.end_of_line()?;
    }
}

/// A single pass over the manifest that records each key and the byte range of each one-line string.
struct Scanner<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
    table: Vec<String>,
    out: Vec<Entry>,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn at(&self, text: &str) -> bool {
        self.b.get(self.i..).is_some_and(|rest| rest.starts_with(text.as_bytes()))
    }

    fn err(&self, what: &str) -> String {
        let before = self.b.get(..self.i).unwrap_or(self.b);
        let breaks = before.iter().enumerate().filter(|&(at, &b)| b == b'\n' || (b == b'\r' && before.get(at + 1) != Some(&b'\n'))).count();
        let line = 1 + breaks;
        format!("Cargo.toml line {line}: {what}")
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", char::from(c))))
        }
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.i += 1;
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            while !matches!(self.peek(), None | Some(b'\n' | b'\r')) {
                self.i += 1;
            }
        }
    }

    /// Skips a line ending: LF, CRLF or a lone CR.
    fn newline(&mut self) -> bool {
        match self.peek() {
            Some(b'\n') => self.i += 1,
            Some(b'\r') => self.i += if self.b.get(self.i + 1) == Some(&b'\n') { 2 } else { 1 },
            _ => return false,
        }
        true
    }

    /// Skips spaces, comments and line endings.
    fn skip_blank(&mut self) -> Result<(), String> {
        loop {
            self.skip_spaces();
            self.skip_comment();
            if !self.newline() {
                return Ok(());
            }
        }
    }

    fn end_of_line(&mut self) -> Result<(), String> {
        self.skip_spaces();
        self.skip_comment();
        if self.peek().is_none() || self.newline() { Ok(()) } else { Err(self.err("expected the end of the line")) }
    }

    /// `[table]` or `[[array.of.tables]]`.
    fn header(&mut self) -> Result<(), String> {
        self.i += 1;
        let array = self.peek() == Some(b'[');
        if array {
            self.i += 1;
        }
        let mut path = self.key()?;
        self.expect(b']')?;
        if array {
            self.expect(b']')?;
            path.push("[]".to_owned());
        }
        self.table = path;
        Ok(())
    }

    fn key_value(&mut self) -> Result<(), String> {
        let key = self.key()?;
        self.expect(b'=')?;
        let mut path = self.table.clone();
        path.extend(key);
        self.value(Some(path), 0)
    }

    /// A dotted key, with the spaces around it.
    fn key(&mut self) -> Result<Vec<String>, String> {
        let mut parts = Vec::new();
        loop {
            self.skip_spaces();
            parts.push(self.simple_key()?);
            self.skip_spaces();
            if self.peek() != Some(b'.') {
                return Ok(parts);
            }
            self.i += 1;
        }
    }

    fn simple_key(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(b'"') if !self.at("\"\"\"") => Ok(self.basic()?.text),
            Some(b'\'') if !self.at("'''") => Ok(self.literal()?.text),
            _ => {
                let start = self.i;
                while self.peek().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                    self.i += 1;
                }
                if start == self.i {
                    return Err(self.err("expected a key"));
                }
                slice(self.s, start..self.i).map(str::to_owned)
            }
        }
    }

    /// A value; it is recorded under `path` unless `path` is `None` (inside an array).
    fn value(&mut self, path: Option<Vec<String>>, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(self.err("arrays and inline tables nest too deeply"));
        }
        self.skip_spaces();
        let value = match self.peek() {
            Some(b'"') if self.at("\"\"\"") => self.long_string(b'"').map(|()| Value::LongStr)?,
            Some(b'\'') if self.at("'''") => self.long_string(b'\'').map(|()| Value::LongStr)?,
            Some(b'"') => Value::Str(self.basic()?),
            Some(b'\'') => Value::Str(self.literal()?),
            Some(b'[') => self.array(depth).map(|()| Value::Other)?,
            Some(b'{') => self.inline_table(path.as_deref(), depth).map(|()| Value::Other)?,
            _ => self.scalar().map(|()| Value::Other)?,
        };
        if let Some(path) = path {
            self.out.push(Entry { path, value });
        }
        Ok(())
    }

    /// A number, boolean or date; only its extent matters.
    fn scalar(&mut self) -> Result<(), String> {
        let start = self.i;
        let ends = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}' | b'#');
        while self.peek().is_some_and(|b| !ends(b)) {
            self.i += 1;
        }
        // A date and a time may be separated by one space: `1979-05-27 07:32:00Z`.
        let date = self.i - start == 10 && self.b.get(start + 4) == Some(&b'-');
        if date && self.peek() == Some(b' ') && self.b.get(self.i + 1).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
            while self.peek().is_some_and(|b| !ends(b)) {
                self.i += 1;
            }
        }
        if start == self.i { Err(self.err("expected a value")) } else { Ok(()) }
    }

    fn array(&mut self, depth: usize) -> Result<(), String> {
        self.i += 1;
        loop {
            self.skip_blank()?;
            if self.peek() == Some(b']') {
                self.i += 1;
                return Ok(());
            }
            self.value(None, depth + 1)?;
            self.skip_blank()?;
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(());
                }
                _ => return Err(self.err("expected `,` or `]` in an array")),
            }
        }
    }

    /// An inline table; line breaks inside it are accepted.
    fn inline_table(&mut self, path: Option<&[String]>, depth: usize) -> Result<(), String> {
        self.i += 1;
        loop {
            self.skip_blank()?;
            if self.peek() == Some(b'}') {
                self.i += 1;
                return Ok(());
            }
            let key = self.key()?;
            self.expect(b'=')?;
            let child = path.map(|p| p.iter().cloned().chain(key).collect());
            self.value(child, depth + 1)?;
            self.skip_blank()?;
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(());
                }
                _ => return Err(self.err("expected `,` or `}` in an inline table")),
            }
        }
    }

    /// A one-line `"…"` string, with its escapes decoded.
    fn basic(&mut self) -> Result<Str, String> {
        let start = self.i;
        self.i += 1;
        let mut text = String::new();
        loop {
            match self.peek() {
                None | Some(b'\n' | b'\r') => return Err(self.err("unterminated string")),
                Some(b'"') => break,
                Some(b'\\') => text.push(self.escape()?),
                Some(_) => {
                    let run = self.i;
                    while self.peek().is_some_and(|b| !matches!(b, b'"' | b'\\' | b'\n' | b'\r')) {
                        self.i += 1;
                    }
                    text.push_str(slice(self.s, run..self.i)?);
                }
            }
        }
        self.i += 1;
        Ok(Str { span: start..self.i, text, quote: b'"' })
    }

    fn escape(&mut self) -> Result<char, String> {
        self.i += 1;
        let c = self.peek().ok_or_else(|| self.err("unterminated string"))?;
        self.i += 1;
        let digits = match c {
            b'"' => return Ok('"'),
            b'\\' => return Ok('\\'),
            b'b' => return Ok('\u{8}'),
            b't' => return Ok('\t'),
            b'n' => return Ok('\n'),
            b'f' => return Ok('\u{c}'),
            b'r' => return Ok('\r'),
            b'e' => return Ok('\u{1b}'),
            b'u' => 4,
            b'U' => 8,
            _ => return Err(self.err("unknown escape in a string")),
        };
        let hex = self.s.get(self.i..self.i + digits).filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
        let ch = hex.and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32).ok_or_else(|| self.err("invalid unicode escape"))?;
        self.i += digits;
        Ok(ch)
    }

    /// A one-line `'…'` string.
    fn literal(&mut self) -> Result<Str, String> {
        let start = self.i;
        self.i += 1;
        while self.peek().is_some_and(|b| !matches!(b, b'\'' | b'\n' | b'\r')) {
            self.i += 1;
        }
        if self.peek() != Some(b'\'') {
            return Err(self.err("unterminated string"));
        }
        let text = slice(self.s, start + 1..self.i)?.to_owned();
        self.i += 1;
        Ok(Str { span: start..self.i, text, quote: b'\'' })
    }

    /// A `"""…"""` or `'''…'''` string; only its extent matters.
    fn long_string(&mut self, q: u8) -> Result<(), String> {
        self.i += 3;
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated multi-line string")),
                Some(b'\\') if q == b'"' => self.i += 2,
                Some(c) if c == q && self.b.get(self.i..self.i + 3) == Some(&[q, q, q][..]) => {
                    self.i += 3;
                    // Up to two more quotes belong to the string.
                    for _ in 0..2 {
                        if self.peek() == Some(q) {
                            self.i += 1;
                        }
                    }
                    return Ok(());
                }
                Some(_) => self.i += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const MANIFEST: &str = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace.dependencies]\nfoo = { version = \"1\" }\n";

    /// A workspace folder holding `manifest` as its `Cargo.toml` and, when given, `lock` as its `Cargo.lock`.
    fn workspace(manifest: &str, lock: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), manifest).unwrap();
        if let Some(lock) = lock {
            std::fs::write(dir.path().join("Cargo.lock"), lock).unwrap();
        }
        dir
    }

    /// The lock directory of the tests' runs: inside the test's own workspace folder, which is removed
    /// with it.
    fn test_lock_dir(root: &Path) -> PathBuf {
        root.join("target").join("xtask-version")
    }

    fn run_limited(root: &Path, args: &[&str], limits: &Limits, update: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        run_to(root, args, limits, Some(&[test_lock_dir(root)]), &mut Vec::new(), update)
    }

    fn run_in(root: &Path, args: &[&str], update: impl FnOnce() -> Result<(), String>) -> (Result<(), String>, String) {
        let mut out = Vec::new();
        let result = run_to(root, args, &Limits::default(), Some(&[test_lock_dir(root)]), &mut out, update);
        (result, String::from_utf8(out).unwrap())
    }

    fn file(root: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(root.join(name)).ok()
    }

    #[test]
    fn reads_the_workspace_version() {
        assert_eq!(read(MANIFEST).unwrap(), "0.1.0");
        assert!(read("[package]\nversion = \"1.0.0\"\n").is_err());
    }

    #[test]
    fn replaces_only_the_workspace_version() {
        let out = replace(MANIFEST, "1.2.3-rc.1").unwrap();
        assert_eq!(read(&out).unwrap(), "1.2.3-rc.1");
        assert_eq!(out, MANIFEST.replace("version = \"0.1.0\"", "version = \"1.2.3-rc.1\""));
        assert!(out.contains("foo = { version = \"1\" }"));
        // Setting the old version back gives the same bytes.
        assert_eq!(replace(&out, "0.1.0").unwrap(), MANIFEST);
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let crlf = MANIFEST.replace('\n', "\r\n");
        let out = replace(&crlf, "2.0.0").unwrap();
        assert_eq!(out, crlf.replace("\"0.1.0\"", "\"2.0.0\""));
    }

    #[test]
    fn validates_versions() {
        for ok in ["0.1.0", "1.20.300", "1.0.0-rc.1", "1.0.0-alpha", "1.0.0-x-y.2"] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
        for bad in ["1.0", "1.0.0.0", "v1.0.0", "01.0.0", "1.0.0-", "1.0.0-a..b", "1.0.0+meta", "1.a.0", ""] {
            assert!(validate(bad).is_err(), "{bad}");
        }
        assert!(replace(MANIFEST, "nope").is_err());
    }

    #[test]
    fn rejects_what_cargo_rejects() {
        for bad in ["1.0.0-01", "1.0.0-rc.01", "1.0.0-0.00", "1.0.0-rc.1.007", "99999999999999999999.0.0", "1.18446744073709551616.0"] {
            assert!(validate(bad).is_err(), "{bad}");
            assert!(replace(MANIFEST, bad).is_err(), "{bad}");
        }
        for ok in ["1.0.0-0", "1.0.0-rc.0", "1.0.0-0a", "1.0.0-01a", "1.0.0-99999999999999999999", "18446744073709551615.0.0"] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn moves_the_workspace_crates_pins_with_it() {
        let manifest = "[workspace.package]\nversion = \"0.1.0\"\n\n[workspace.dependencies]\n\
                        own = { path = \"crates/own\", version = \"0.1.0\" }\n\
                        exact = { path = \"crates/exact\", version = \"=0.1.0\" }\n\
                        dep = { version = \"0.1.0\" }\n\
                        other = { path = \"crates/other\", version = \"0.2\" }\n\
                        newer = { path = \"crates/newer\", version = \"0.1.0-rc.1\" }\n";
        let out = replace(manifest, "0.2.0").unwrap();
        assert!(out.contains("own = { path = \"crates/own\", version = \"0.2.0\" }"), "{out}");
        assert!(out.contains("exact = { path = \"crates/exact\", version = \"=0.2.0\" }"), "{out}");
        assert!(out.contains("dep = { version = \"0.1.0\" }"), "a registry dependency keeps its version");
        assert!(out.contains("other = { path = \"crates/other\", version = \"0.2\" }"));
        assert!(out.contains("newer = { path = \"crates/newer\", version = \"0.1.0-rc.1\" }"));
    }

    #[test]
    fn a_failed_update_restores_the_manifest() {
        // Two sets in one workspace: a child started by another test must not hold the first one's lock.
        let _guard = crate::spawn_guard();
        let dir = workspace(MANIFEST, None);
        let (err, out) = run_in(dir.path(), &["set", "2.0.0"], || Err("cargo update failed".into()));
        let err = err.unwrap_err();
        assert!(err.contains("restored to 0.1.0"), "{err}");
        assert_eq!(out, "", "a failed set printed success");
        assert_eq!(file(dir.path(), "Cargo.toml").as_deref(), Some(MANIFEST));
        let (ok, out) = run_in(dir.path(), &["set", "2.0.0"], || Ok(()));
        assert_eq!(ok, Ok(()));
        assert_eq!(out, "version: 0.1.0 -> 2.0.0\n");
        assert_eq!(read(&file(dir.path(), "Cargo.toml").unwrap()).unwrap(), "2.0.0");
    }

    #[test]
    fn the_real_manifest_has_a_valid_version() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml")).unwrap();
        let version = read(&text).unwrap();
        validate(&version).unwrap();
        // craft-libs pins its own crates with `version = "=X.Y.Z"`; a set moves those pins too.
        let out = replace(&text, "99.0.0").unwrap();
        let pins = text.matches(&format!("version = \"={version}\"")).count();
        assert!(pins > 0, "the root manifest has no `=` pin of its own version");
        assert_eq!(out.matches("version = \"=99.0.0\"").count(), pins, "{out}");
        assert_eq!(replace(&out, &version).unwrap(), text);
    }

    #[test]
    fn reads_commented_and_single_quoted_versions() {
        let manifest = "[workspace.package]  # shared\nversion = '1.2.3' # the release\nedition = \"2024\"\n";
        assert_eq!(read(manifest).unwrap(), "1.2.3");
        let out = replace(manifest, "1.3.0").unwrap();
        assert_eq!(out, "[workspace.package]  # shared\nversion = '1.3.0' # the release\nedition = \"2024\"\n");
        let spaced = "[ workspace . package ]\n\"version\"=\"1.2.3\"#c\n";
        assert_eq!(replace(spaced, "1.2.4").unwrap(), "[ workspace . package ]\n\"version\"=\"1.2.4\"#c\n");
    }

    #[test]
    fn accepts_dotted_workspace_package_version() {
        for manifest in [
            "[workspace]\npackage.version = \"0.3.0\"\n",
            "workspace.package.version = \"0.3.0\"\n",
            "[workspace]\npackage = { version = \"0.3.0\", edition = \"2024\" }\n",
        ] {
            assert_eq!(read(manifest).unwrap(), "0.3.0", "{manifest}");
            assert_eq!(replace(manifest, "0.4.0").unwrap(), manifest.replace("0.3.0", "0.4.0"));
        }
    }

    #[test]
    fn leaves_member_version_workspace_alone() {
        let manifest = "[package]\nname = \"a\"\nversion.workspace = true\n\n[workspace.package]\nversion = \"0.1.0\"\n\n\
                        [workspace.dependencies]\na = { path = \"a\", version.workspace = true }\n";
        let out = replace(manifest, "0.2.0").unwrap();
        assert_eq!(out, manifest.replace("version = \"0.1.0\"", "version = \"0.2.0\""));
        assert!(read("[package]\nversion.workspace = true\n").is_err());
    }

    #[test]
    fn moves_multiline_path_dependency_pin() {
        let manifest = "[workspace.package]\nversion = \"0.1.0\"\n\n\
                        [workspace.dependencies]\n\
                        spread = {\n  path = \"crates/spread\",\n  version = \"0.1.0\",\n}\n\
                        dotted.path = \"crates/dotted\"\ndotted.version = \"0.1.0\"\n\n\
                        [workspace.dependencies.own]\nversion = \"0.1.0\"\n# the crate\npath = \"crates/own\"\n\n\
                        [workspace.dependencies.registry]\nversion = \"0.1.0\"\n";
        let out = replace(manifest, "0.1.1").unwrap();
        let expected = manifest.replacen("0.1.0", "0.1.1", 4);
        assert_eq!(out, expected);
        assert!(out.ends_with("[workspace.dependencies.registry]\nversion = \"0.1.0\"\n"), "{out}");
    }

    #[test]
    fn preserves_mixed_and_cr_line_endings() {
        let mixed = "[workspace]\r\nmembers = [\r\"a\",\n]\r\n[workspace.package]\rversion = \"0.1.0\"\r\n# end\r";
        let out = replace(mixed, "0.2.0").unwrap();
        assert_eq!(out, mixed.replace("0.1.0", "0.2.0"));
        let cr = MANIFEST.replace('\n', "\r");
        assert_eq!(read(&cr).unwrap(), "0.1.0");
        assert_eq!(replace(&cr, "2.0.0").unwrap(), cr.replace("\"0.1.0\"", "\"2.0.0\""));
    }

    #[test]
    fn ignores_brackets_and_keys_inside_strings_and_arrays() {
        let manifest = "[workspace]\nmembers = [\n  [\"nested\"],\n  \"b\",\n]\nnote = \"\"\"\n[workspace.package]\nversion = \"9.9.9\"\n\"\"\"\n\
                        title = '[workspace.package]'\n\n[workspace.package]\nversion = \"0.1.0\"\n";
        assert_eq!(read(manifest).unwrap(), "0.1.0");
        assert_eq!(replace(manifest, "0.2.0").unwrap(), manifest.replace("\"0.1.0\"", "\"0.2.0\""));
    }

    #[test]
    fn rejects_unsupported_and_ambiguous_manifests() {
        let deep = format!("[workspace.package]\nversion = \"0.1.0\"\nx = {}{}\n", "[".repeat(100), "]".repeat(100));
        for (manifest, what) in [
            ("[workspace.package]\nversion = \"0.1.0\"\n[workspace]\npackage.version = \"0.2.0\"\n", "more than once"),
            ("[workspace.package]\nversion = \"\"\"0.1.0\"\"\"\n", "multi-line"),
            ("[workspace.package]\nversion = 1\n", "not a string"),
            ("[workspace.package]\nversion = \"\"\n", "empty"),
            ("[workspace.package]\nversion = \"0.1.0\n", "unterminated"),
            ("[workspace.package]\nversion = \"0.1.0\" extra\n", "end of the line"),
            ("[workspace.package\nversion = \"0.1.0\"\n", "expected `]`"),
            ("[workspace.package]\nversion = \"\\q\"\n", "escape"),
            (deep.as_str(), "nest too deeply"),
        ] {
            let Err(e) = replace(manifest, "1.0.0") else { panic!("accepted {manifest:?}") };
            assert!(e.contains(what), "{manifest:?}: {e}");
        }
        let Err(e) = replace("[workspace.package]\nversion = \"1.0\"\n", "1.0.1") else { panic!("an invalid current version was replaced") };
        assert!(e.contains("current"), "{e}");
    }

    #[test]
    fn rejects_oversized_manifest_before_write() {
        let dir = workspace(MANIFEST, None);
        let ran = Cell::new(false);
        let limits = Limits::default().with_max_manifest_bytes(MANIFEST.len() as u64 - 1);
        let result = run_limited(dir.path(), &["set", "2.0.0"], &limits, || {
            ran.set(true);
            Ok(())
        });
        let Err(e) = result else { panic!("an oversized manifest was read") };
        assert!(e.contains("limit"), "{e}");
        assert!(!ran.get());
        assert_eq!(file(dir.path(), "Cargo.toml").as_deref(), Some(MANIFEST));
        let exact = Limits::default().with_max_manifest_bytes(MANIFEST.len() as u64);
        assert_eq!(run_limited(dir.path(), &[], &exact, || Ok(())), Ok(()));

        let dir = workspace(MANIFEST, Some("lock"));
        let small_lock = Limits::default().with_max_lock_bytes(3);
        let Err(e) = run_limited(dir.path(), &["set", "2.0.0"], &small_lock, || Ok(())) else { panic!("an oversized lock was read") };
        assert!(e.contains("Cargo.lock") && e.contains("limit"), "{e}");
        assert_eq!(file(dir.path(), "Cargo.toml").as_deref(), Some(MANIFEST));
    }

    #[test]
    fn restores_lockfile_after_failed_update() {
        let dir = workspace(MANIFEST, Some("old lock\r\n"));
        let lock = dir.path().join("Cargo.lock");
        let (err, _) = run_in(dir.path(), &["set", "2.0.0"], || {
            std::fs::write(&lock, "half-updated lock").unwrap();
            Err("cargo update failed".into())
        });
        assert!(err.unwrap_err().contains("restored to 0.1.0"));
        assert_eq!(file(dir.path(), "Cargo.lock").as_deref(), Some("old lock\r\n"));
        assert_eq!(file(dir.path(), "Cargo.toml").as_deref(), Some(MANIFEST));

        // A lock that the failed update created is removed again.
        let dir = workspace(MANIFEST, None);
        let lock = dir.path().join("Cargo.lock");
        let (err, _) = run_in(dir.path(), &["set", "2.0.0"], || {
            std::fs::write(&lock, "new lock").unwrap();
            Err("cargo update failed".into())
        });
        assert!(err.is_err());
        assert!(!lock.exists());
    }

    #[test]
    fn setting_current_version_does_not_run_cargo() {
        let dir = workspace(MANIFEST, None);
        let ran = Cell::new(false);
        let (result, out) = run_in(dir.path(), &["set", "0.1.0"], || {
            ran.set(true);
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert_eq!(out, "version: 0.1.0 (unchanged)\n");
        assert!(!ran.get());
        assert_eq!(file(dir.path(), "Cargo.toml").as_deref(), Some(MANIFEST));
        assert_eq!(file(dir.path(), "Cargo.lock"), None);
    }

    #[test]
    fn lock_dirs_cover_the_workspace_and_the_account() {
        let (a, b) = (workspace(MANIFEST, None), workspace(MANIFEST, None));
        let dirs_a = lock_dirs(a.path()).unwrap();
        assert_eq!(lock_dirs(&a.path().join(".")).unwrap(), dirs_a, "two spellings of one root");
        assert_ne!(lock_dirs(b.path()).unwrap(), dirs_a);
        let canonical = std::fs::canonicalize(a.path()).unwrap();
        assert_eq!(dirs_a[0].parent(), Some(canonical.join("target").join("xtask-version").as_path()));
        // Both directories are named for the workspace, so a `target` shared by two workspaces keeps
        // their locks apart.
        let name = dirs_a[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("version-"), "{name}");
        assert!(dirs_a.iter().all(|d| d.file_name() == dirs_a[0].file_name()));
        for dir in &dirs_a[1..] {
            assert!(dir.is_absolute() && dir.parent().is_some_and(|p| p.ends_with("craft-xtask")), "{}", dir.display());
            assert!(!dir.starts_with(&canonical), "{}", dir.display());
        }
        assert!(dirs_a.len() <= 2);
        assert!(lock_dirs(&a.path().join("missing")).is_err());
        // FNV-1a's published test value, so the name stays the same across builds.
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn a_held_workspace_lock_refuses_the_set_whatever_the_lock_directories() {
        let _guard = crate::spawn_guard();
        let dir = workspace(MANIFEST, Some("old lock\n"));
        let other = base_lock(dir.path(), &test_lock_dir(dir.path())).unwrap().expect("free lock");
        // A run whose owner-labeled lock directories differ from the holder's still meets its lock. On
        // Windows that lock is `workspace.lock` in the first directory, so only the later ones differ.
        let elsewhere = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        let dirs = [test_lock_dir(dir.path()), elsewhere.path().join("lock")];
        #[cfg(not(windows))]
        let dirs = [elsewhere.path().join("lock")];
        let ran = Cell::new(false);
        let result = run_to(dir.path(), &["set", "2.0.0"], &Limits::default(), Some(&dirs), &mut Vec::new(), || {
            ran.set(true);
            Ok(())
        });
        assert!(result.is_err_and(|e| e.contains("another `cargo xtask version set`")));
        assert!(!ran.get());
        assert_eq!(file(dir.path(), "Cargo.toml").as_deref(), Some(MANIFEST));
        assert_eq!(file(dir.path(), "Cargo.lock").as_deref(), Some("old lock\n"));
        drop(other);
        assert_eq!(run_in(dir.path(), &["set", "2.0.0"], || Ok(())).0, Ok(()));
    }

    #[test]
    fn the_workspace_lock_is_required_and_a_later_one_is_optional() {
        let _guard = crate::spawn_guard();
        let dir = workspace(MANIFEST, None);
        // A directory cannot be made under a file.
        let blocked = dir.path().join("Cargo.toml").join("lock");
        let first = test_lock_dir(dir.path());
        let result = run_to(dir.path(), &["set", "2.0.0"], &Limits::default(), Some(&[first.clone(), blocked.clone()]), &mut Vec::new(), || Ok(()));
        assert_eq!(result, Ok(()));
        assert!(first.join("ci.lock").exists());

        // A run holding the lock in a later directory refuses the set too.
        let later = dir.path().join("later");
        let other = crate::lock::try_acquire(&crate::lock::LockConfig::new(&later, 1, "pid 8 in /w: cargo xtask version set 4.0.0"))
            .unwrap()
            .expect("free lock");
        let ran = Cell::new(false);
        let dirs = [first.clone(), blocked.clone(), later.clone()];
        let result = run_to(dir.path(), &["set", "3.0.0"], &Limits::default(), Some(&dirs), &mut Vec::new(), || {
            ran.set(true);
            Ok(())
        });
        assert!(result.is_err_and(|e| e.contains("pid 8 in /w")));
        assert!(!ran.get());
        assert_eq!(read(&file(dir.path(), "Cargo.toml").unwrap()).unwrap(), "2.0.0");
        drop(other);
        // The lock taken in the first directory before the refusal was released.
        assert!(crate::lock::try_acquire(&crate::lock::LockConfig::new(&first, 1, "x")).unwrap().is_some());

        // Without the first lock, nothing is written.
        let dir = workspace(MANIFEST, None);
        let blocked = dir.path().join("Cargo.toml").join("lock");
        let ran = Cell::new(false);
        let result = run_to(dir.path(), &["set", "2.0.0"], &Limits::default(), Some(&[blocked, test_lock_dir(dir.path())]), &mut Vec::new(), || {
            ran.set(true);
            Ok(())
        });
        assert!(result.is_err_and(|e| e.contains("could not take the version lock")));
        assert!(!ran.get());
        assert_eq!(file(dir.path(), "Cargo.toml").as_deref(), Some(MANIFEST));
    }

    #[test]
    fn prints_the_version_and_rejects_other_words() {
        let dir = workspace(MANIFEST, None);
        assert_eq!(run_in(dir.path(), &[], || Ok(())), (Ok(()), "0.1.0\n".to_owned()));
        for args in [&["set"][..], &["set", "1.0.0", "2.0.0"], &["get"], &["set", "1.0.0+meta"]] {
            let (result, out) = run_in(dir.path(), args, || panic!("cargo ran for {args:?}"));
            assert!(result.is_err(), "{args:?}");
            assert_eq!(out, "");
        }
        assert_eq!(run_in(dir.path(), &["get"], || Ok(())).0, Err(USAGE.to_owned()));
        assert!(run_in(&dir.path().join("missing"), &[], || Ok(())).0.unwrap_err().starts_with("read "));
    }

    #[test]
    fn leaves_no_staging_files() {
        let _guard = crate::spawn_guard();
        let dir = workspace(MANIFEST, Some("lock"));
        let names = || {
            let mut n: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            n.sort();
            n
        };
        let lock_files = || {
            std::fs::read_dir(test_lock_dir(dir.path())).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>()
        };
        let no_owner = |files: Vec<String>| files.iter().all(|f| !f.ends_with(".owner"));
        let _ = run_in(dir.path(), &["set", "2.0.0"], || Err("cargo update failed".into()));
        assert_eq!(names(), ["Cargo.lock", "Cargo.toml", "target"]);
        assert!(no_owner(lock_files()), "the owner file outlived the run: {:?}", lock_files());
        assert_eq!(run_in(dir.path(), &["set", "2.0.0"], || Ok(())).0, Ok(()));
        assert_eq!(names(), ["Cargo.lock", "Cargo.toml", "target"]);
        assert!(no_owner(lock_files()), "{:?}", lock_files());
    }

    #[test]
    fn concurrent_set_is_refused_before_anything_is_written() {
        let _guard = crate::spawn_guard();
        let dir = workspace(MANIFEST, Some("old lock\n"));
        let config = crate::lock::LockConfig::new(test_lock_dir(dir.path()), 1, "pid 7 in /w: cargo xtask version set 9.0.0");
        let other = crate::lock::try_acquire(&config).unwrap().expect("free lock");
        let ran = Cell::new(false);
        let (result, out) = run_in(dir.path(), &["set", "2.0.0"], || {
            ran.set(true);
            Err("cargo update failed".into())
        });
        let err = result.unwrap_err();
        assert!(err.contains("another `cargo xtask version set`") && err.contains("pid 7 in /w"), "{err}");
        assert_eq!(out, "");
        assert!(!ran.get(), "cargo update ran while another set held the lock");
        assert_eq!(file(dir.path(), "Cargo.toml").as_deref(), Some(MANIFEST));
        assert_eq!(file(dir.path(), "Cargo.lock").as_deref(), Some("old lock\n"));
        // Reading the version does not take the lock. Setting the current one does: the other run may
        // still put an older version back.
        assert_eq!(run_in(dir.path(), &[], || Ok(())), (Ok(()), "0.1.0\n".to_owned()));
        let (unchanged, out) = run_in(dir.path(), &["set", "0.1.0"], || Ok(()));
        assert!(unchanged.is_err_and(|e| e.contains("pid 7 in /w")), "{out}");
        assert_eq!(out, "");
        // An invalid version is refused before the lock.
        assert!(run_in(dir.path(), &["set", "nope"], || Ok(())).0.unwrap_err().contains("not a version"));
        drop(other);
        assert_eq!(run_in(dir.path(), &["set", "2.0.0"], || Ok(())), (Ok(()), "version: 0.1.0 -> 2.0.0\n".to_owned()));
    }

    #[test]
    fn replaces_the_existing_manifest() {
        let dir = workspace(MANIFEST, None);
        // An open reader of the old manifest does not stop the replacement.
        let reader = File::open(dir.path().join("Cargo.toml")).unwrap();
        assert_eq!(run_in(dir.path(), &["set", "3.0.0"], || Ok(())).0, Ok(()));
        drop(reader);
        assert_eq!(read(&file(dir.path(), "Cargo.toml").unwrap()).unwrap(), "3.0.0");
    }
}
