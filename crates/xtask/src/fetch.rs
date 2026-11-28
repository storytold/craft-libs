//! Pinned downloads.
//!
//! [`fetch_pinned`] puts each file of a pinned list under a destination folder. A file that is
//! already there with the pinned SHA-256 is kept; any other file at that name is deleted and
//! downloaded again. A download is written to a temporary file beside its destination, counted
//! against [`Pin::max_bytes`] and hashed while it is written, and renamed into place only when its
//! SHA-256 matches, so a failed or rejected download leaves no file at the name.
//!
//! [`fetch_archive`] downloads one pinned tar archive (plain or gzip-compressed), checks its
//! SHA-256, reads it in memory under [`ExtractOptions`], and writes its regular files and folders
//! under the destination. Every entry is checked before the first file is written: links, devices
//! and other special entries, absolute paths, `..`, names that differ only in case or in Unicode
//! normalization, and names that Windows cannot hold are refused.
//!
//! The pinned list and the destination come from the caller. Only `https://` addresses are
//! accepted. [`Curl`] runs the system `curl`, limited to HTTPS for the address and every redirect;
//! a [`Transport`] supplied by the caller replaces it, for example in tests.
//!
//! Neither the destination nor any path under it may be a link: a link where the destination, a
//! folder or a file goes is refused with [`FetchError::NotRegular`]. The folders above the
//! destination are the caller's path and are followed like any other path, links included (on
//! macOS the system temporary folder is under the link `/var`).

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};
use tar::EntryType;
use unicode_normalization::UnicodeNormalization;

/// Default for [`ExtractOptions::max_entries`].
pub const DEFAULT_MAX_ENTRIES: usize = 1000;
/// Default for [`ExtractOptions::max_entry_bytes`]: 4 MiB.
pub const DEFAULT_MAX_ENTRY_BYTES: u64 = 4 << 20;
/// Default for [`ExtractOptions::max_expanded_bytes`]: 16 MiB.
pub const DEFAULT_MAX_EXPANDED_BYTES: u64 = 16 << 20;

/// Longest name accepted, in bytes.
const MAX_NAME_BYTES: usize = 4096;
/// Longest part of a name accepted, in bytes: the limit of ext4 and APFS, and no more than the 255
/// UTF-16 units of NTFS.
const MAX_PART_BYTES: usize = 255;
/// Tar data allowed per entry beyond the file bytes: its header (512) and padding (511), and a GNU
/// long-name record and a pax record, each a 512-byte header with up to 4,608 bytes of padded data
/// for a name of [`MAX_NAME_BYTES`].
const ENTRY_OVERHEAD: u64 = 12 * 1024;

/// Names Windows keeps for devices, whatever the extension.
const RESERVED: [&str; 28] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// One file of a pinned list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin<'a> {
    /// Path of the file under the destination, with `/` between folders. For
    /// [`fetch_archive`], the archive's name in messages; the archive itself is not written.
    pub name: &'a str,
    /// The file's `https://` address.
    pub url: &'a str,
    /// The file's SHA-256: 64 hexadecimal digits, in either case.
    pub sha256: &'a str,
    /// The most bytes accepted for the file; at least 1.
    pub max_bytes: u64,
}

/// What [`fetch_pinned`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchSummary {
    /// Files that were already in place with the pinned SHA-256.
    pub reused: usize,
    /// Files downloaded and written.
    pub downloaded: usize,
}

/// Limits for [`fetch_archive`] and [`extract`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractOptions {
    /// The most entries the archive may hold, folders included.
    pub max_entries: usize,
    /// The most bytes of one file.
    pub max_entry_bytes: u64,
    /// The most bytes of all files together.
    pub max_expanded_bytes: u64,
    /// Leading folders removed from every entry's path, as `tar --strip-components`; entries with
    /// no more than this many parts are skipped.
    pub strip_components: usize,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_ENTRIES,
            max_entry_bytes: DEFAULT_MAX_ENTRY_BYTES,
            max_expanded_bytes: DEFAULT_MAX_EXPANDED_BYTES,
            strip_components: 0,
        }
    }
}

/// Why a fetch stopped.
#[non_exhaustive]
#[derive(Debug)]
pub enum FetchError {
    /// A pin or an archive entry was refused before anything was written.
    Invalid { name: String, reason: String },
    /// A path under the destination is a link, a folder where a file goes, a file where a folder
    /// goes, or another kind of file.
    NotRegular(PathBuf),
    /// A download or an archive went past a limit.
    TooLarge { name: String, limit: String },
    /// The downloaded bytes have another SHA-256 than the pin.
    HashMismatch { name: String, expected: String, actual: String },
    /// The download failed.
    Download { name: String, source: io::Error },
    /// The archive could not be read.
    Archive { name: String, reason: String },
    /// Reading or writing under the destination failed.
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // A name may hold control characters (a refused pin, an archive entry), so it is escaped.
            FetchError::Invalid { name, reason } => write!(f, "{}: {reason}", name.escape_debug()),
            FetchError::NotRegular(path) => write!(f, "{} is a link or another kind of file than expected", path.display()),
            FetchError::TooLarge { name, limit } => write!(f, "{} is over its limit of {limit}", name.escape_debug()),
            FetchError::HashMismatch { name, expected, actual } => {
                write!(f, "{}: SHA-256 is {actual}, but the pin is {expected}", name.escape_debug())
            }
            FetchError::Download { name, source } => write!(f, "downloading {} failed: {source}", name.escape_debug()),
            FetchError::Archive { name, reason } => write!(f, "{}: {reason}", name.escape_debug()),
            FetchError::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for FetchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FetchError::Download { source, .. } | FetchError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Sends the body found at an address to a writer.
pub trait Transport {
    /// Writes the body of `url` to `out`. `max_bytes` is the most the caller accepts; the caller
    /// counts the bytes itself and stops the transfer with a write error past it. A transport that
    /// refuses the body itself for being over `max_bytes` returns an error of kind
    /// [`io::ErrorKind::FileTooLarge`], which the caller reports as [`FetchError::TooLarge`].
    fn download(&self, url: &str, max_bytes: u64, out: &mut dyn Write) -> io::Result<()>;
}

/// The [`Transport`] of [`fetch_pinned`] and [`fetch_archive`]: the system `curl`, reading no
/// configuration file, requesting the address literally (no `[]` or `{}` globbing), allowed HTTPS
/// only for the address and every redirect, failing on an HTTP error status, refusing a body whose
/// advertised size is over the limit before it arrives, and stopping a transfer that stays under
/// 1 KiB per second for 60 seconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Curl;

impl Transport for Curl {
    fn download(&self, url: &str, max_bytes: u64, out: &mut dyn Write) -> io::Result<()> {
        let mut child = Command::new("curl")
            .args(curl_args(url, max_bytes))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("cannot run curl: {e}")))?;
        let copied = match child.stdout.take() {
            Some(mut body) => io::copy(&mut body, out).map(drop),
            None => Err(io::Error::other("curl's output cannot be read")),
        };
        if let Err(e) = copied {
            // A limit or a write failed, so curl is stopped.
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        let status = child.wait()?;
        match status.code() {
            Some(0) => Ok(()),
            // Exit status 63: `--max-filesize` refused a body whose advertised size is over the limit.
            Some(63) => Err(io::Error::new(io::ErrorKind::FileTooLarge, format!("curl refused a body over {max_bytes} bytes"))),
            _ => Err(io::Error::other(format!("curl failed: {status}"))),
        }
    }
}

/// The arguments [`Curl`] passes: `-q` first, so no `.curlrc` changes them.
fn curl_args(url: &str, max_bytes: u64) -> Vec<String> {
    let mut args: Vec<String> = [
        "-q",
        "--globoff",
        "--path-as-is",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "10",
        "--speed-limit",
        "1024",
        "--speed-time",
        "60",
        "--max-filesize",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    // `curl` reads the limit as a signed 64-bit number; the crate's own count enforces the rest.
    args.push(max_bytes.min(i64::MAX.unsigned_abs()).to_string());
    args.extend(["--output", "-", "--url", url].iter().map(|s| (*s).to_string()));
    args
}

/// Downloads every file of `pins` that is not already under `dest` with its pinned SHA-256,
/// through the system `curl`. See [`fetch_pinned_with`].
pub fn fetch_pinned(dest: &Path, pins: &[Pin<'_>]) -> Result<FetchSummary, FetchError> {
    fetch_pinned_with(dest, pins, &Curl)
}

/// Downloads every file of `pins` that is not already under `dest` with its pinned SHA-256,
/// through `transport`.
///
/// Every pin is checked before anything is read or written: its name is a relative path of
/// portable parts that no other pin repeats (ignoring case and Unicode normalization) or uses as a
/// folder, its address
/// starts with `https://`, its SHA-256 has 64 hexadecimal digits and its `max_bytes` is not 0.
/// `dest` and the folders the names need are created. The files are then handled in order, and
/// the first failure stops the fetch; files handled before it stay in place. As with [`extract`],
/// no folder on the way to a written file may be writable by another user.
pub fn fetch_pinned_with(dest: &Path, pins: &[Pin<'_>], transport: &dyn Transport) -> Result<FetchSummary, FetchError> {
    let checked = check_pins(pins)?;
    prepare(dest)?;
    let mut summary = FetchSummary::default();
    for item in &checked {
        let target = walk(dest, &item.parts, true)?;
        if cached(&target, item)? {
            summary.reused += 1;
        } else {
            download_file(&target, item, transport)?;
            summary.downloaded += 1;
        }
    }
    Ok(summary)
}

/// Downloads the tar archive `archive` through the system `curl` and writes its files under
/// `dest`. See [`fetch_archive_with`].
pub fn fetch_archive(dest: &Path, archive: &Pin<'_>, options: &ExtractOptions) -> Result<Vec<String>, FetchError> {
    fetch_archive_with(dest, archive, options, &Curl)
}

/// Downloads the tar archive `archive` through `transport`, checks its size and SHA-256, and
/// writes its files under `dest` as [`extract`] does. The archive is held in memory;
/// `archive.max_bytes` limits its downloaded size, and the files' data is held as well, up to
/// `options.max_expanded_bytes`. `archive.name` must not be empty, longer than 4096 bytes or hold
/// a control character. Returns the names of the files written.
pub fn fetch_archive_with(dest: &Path, archive: &Pin<'_>, options: &ExtractOptions, transport: &dyn Transport) -> Result<Vec<String>, FetchError> {
    check_label(archive.name).map_err(|reason| invalid(archive.name, reason))?;
    let sha256 = check_fields(archive).map_err(|reason| invalid(archive.name, reason))?;
    let mut sink = Sink::new(Vec::new(), archive.max_bytes);
    let got = transport.download(archive.url, archive.max_bytes, &mut sink);
    let bytes = finish(archive.name, &sha256, sink, got)?;
    extract(dest, archive.name, &bytes, options)
}

/// Writes the regular files and folders of the tar archive `bytes` (plain, or gzip-compressed
/// when it starts with the gzip signature) under `dest`, and returns the names of the files
/// written. `name` labels the archive in errors.
///
/// The whole archive is read and checked before anything is written: an entry count, file size
/// or total over `options`, a link, device or other special entry, a path that is absolute,
/// holds `..`, is not UTF-8 or is not portable, a file name repeated ignoring case and Unicode
/// normalization, and a name
/// used for both a file and a folder each stop it with nothing written. Each file is then written
/// through a temporary file and a rename; an existing file at that name is replaced. Files are
/// written one at a time, so a write error after the checks leaves the files written before it in
/// place. The link checks run before the writes, not as part of them, so no folder on the way to a
/// written file (`dest`, the folders above it and the folders under it) may be writable by another
/// user.
pub fn extract(dest: &Path, name: &str, bytes: &[u8], options: &ExtractOptions) -> Result<Vec<String>, FetchError> {
    let items = read_tar(name, bytes, options)?;
    prepare(dest)?;
    for item in &items {
        check_target(dest, item)?;
    }
    let mut written = Vec::new();
    for item in &items {
        let path = walk(dest, &item.parts, true)?;
        match &item.data {
            Some(data) => {
                craft_atomic::atomic_write(&path, data).map_err(|source| FetchError::Io { path: path.clone(), source })?;
                written.push(item.parts.join("/"));
            }
            None => {
                ensure_dir(&path, true)?;
            }
        }
    }
    Ok(written)
}

/// The lowercase hexadecimal SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(digest: &[u8]) -> String {
    use std::fmt::Write as _;
    digest.iter().fold(String::with_capacity(digest.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn invalid(name: &str, reason: String) -> FetchError {
    FetchError::Invalid { name: name.to_string(), reason }
}

fn too_large(name: &str, limit: String) -> FetchError {
    FetchError::TooLarge { name: name.to_string(), limit }
}

fn io_error(path: &Path, source: io::Error) -> FetchError {
    FetchError::Io { path: path.to_path_buf(), source }
}

/// A pin after its checks: its name split into parts and its SHA-256 in lowercase.
struct Checked<'a> {
    pin: &'a Pin<'a>,
    parts: Vec<&'a str>,
    sha256: String,
}

fn check_pins<'a>(pins: &'a [Pin<'a>]) -> Result<Vec<Checked<'a>>, FetchError> {
    let mut names = Names::default();
    pins.iter()
        .map(|pin| {
            let parts = split_name(pin.name).map_err(|reason| invalid(pin.name, reason))?;
            names.add_file(&parts).map_err(|reason| invalid(pin.name, reason))?;
            let sha256 = check_fields(pin).map_err(|reason| invalid(pin.name, reason))?;
            Ok(Checked { pin, parts, sha256 })
        })
        .collect()
}

/// Checks a pin's address, SHA-256 and limit; returns the SHA-256 in lowercase.
fn check_fields(pin: &Pin<'_>) -> Result<String, String> {
    let host = pin.url.get(..8).filter(|scheme| scheme.eq_ignore_ascii_case("https://")).and_then(|_| pin.url.get(8..));
    match host {
        None => return Err(format!("the address {:?} does not start with https://", pin.url)),
        Some(rest) if rest.is_empty() || rest.starts_with('/') => return Err(format!("the address {:?} has no host", pin.url)),
        Some(_) => {}
    }
    if pin.url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the address holds a space or a control character".into());
    }
    if pin.sha256.len() != 64 || !pin.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("the SHA-256 {:?} is not 64 hexadecimal digits", pin.sha256));
    }
    if pin.max_bytes == 0 {
        return Err("max_bytes is 0".into());
    }
    Ok(pin.sha256.to_ascii_lowercase())
}

/// Checks the name that labels an archive in messages: not empty, within [`MAX_NAME_BYTES`] and
/// free of control characters.
fn check_label(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("the name is empty".into());
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(format!("the name is longer than {MAX_NAME_BYTES} bytes"));
    }
    if let Some(c) = name.chars().find(|c| c.is_control()) {
        return Err(format!("the name holds {c:?}, a control character"));
    }
    Ok(())
}

/// Splits a relative `/`-separated name into its parts and checks each one.
fn split_name(name: &str) -> Result<Vec<&str>, String> {
    if name.is_empty() {
        return Err("the name is empty".into());
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(format!("the name is longer than {MAX_NAME_BYTES} bytes"));
    }
    if name.starts_with('/') {
        return Err("the name is an absolute path".into());
    }
    let parts: Vec<&str> = name.split('/').collect();
    for part in &parts {
        check_part(part)?;
    }
    Ok(parts)
}

fn check_part(part: &str) -> Result<(), String> {
    if part.is_empty() {
        return Err("the name has an empty part (a doubled or trailing `/`)".into());
    }
    if part == "." || part == ".." {
        return Err(format!("the name has a `{part}` part"));
    }
    if part.len() > MAX_PART_BYTES {
        return Err(format!("a part of the name is longer than {MAX_PART_BYTES} bytes"));
    }
    if let Some(c) = part.chars().find(|&c| c.is_control() || matches!(c, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')) {
        return Err(format!("the name holds {c:?}, which is not portable"));
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return Err(format!("the part {part:?} ends with a dot or a space"));
    }
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    if RESERVED.iter().any(|r| stem.eq_ignore_ascii_case(r)) {
        return Err(format!("the part {part:?} is a device name on Windows"));
    }
    Ok(())
}

/// File and folder names seen so far, so that names differing only in case, which are one file or
/// folder on macOS and Windows, or only in Unicode normalization, which are one name on macOS, are
/// refused. Names are compared by their [`fold`] key.
///
/// Each folder is kept once, as its last part under the number of the folder that holds it, and
/// each file as its last part under its folder's number. Case mapping makes a key at most three
/// times as long as its part and normalization at most three times longer again, so the text kept
/// is at most ten times the length of the names added, however deep they are: for an archive, at
/// most `10 × max_entries × 4096` bytes.
#[derive(Default)]
struct Names {
    /// Each file by its folder's number (0 for the destination) and its last part's key.
    files: HashSet<(usize, String)>,
    /// Each folder by its parent's number and its last part's key, mapped to its own number and
    /// its last part as first written.
    folders: HashMap<(usize, String), (usize, String)>,
}

impl Names {
    fn add_file<S: AsRef<str>>(&mut self, parts: &[S]) -> Result<(), String> {
        let Some((last, above)) = parts.split_last() else {
            return Ok(());
        };
        let parent = self.add_parents(above)?;
        let key = (parent, fold(last.as_ref()));
        if self.folders.contains_key(&key) {
            return Err("the name is also used as a folder".into());
        }
        if !self.files.insert(key) {
            return Err("the name appears twice (names are compared ignoring case and Unicode normalization)".into());
        }
        Ok(())
    }

    fn add_folder<S: AsRef<str>>(&mut self, parts: &[S]) -> Result<(), String> {
        let Some((last, above)) = parts.split_last() else {
            return Ok(());
        };
        let parent = self.add_parents(above)?;
        let key = (parent, fold(last.as_ref()));
        if self.files.contains(&key) {
            return Err("the name is also used as a file".into());
        }
        self.insert_folder(key, parts, above.len()).map(drop)
    }

    /// Records each folder of `parts`, each inside the one before it, and returns the last one's
    /// number (0 when `parts` is empty).
    fn add_parents<S: AsRef<str>>(&mut self, parts: &[S]) -> Result<usize, String> {
        let mut parent = 0;
        for (i, part) in parts.iter().enumerate() {
            let key = (parent, fold(part.as_ref()));
            if self.files.contains(&key) {
                return Err(format!("the name is inside {:?}, which is also a file", join(parts.get(..=i).unwrap_or_default())));
            }
            parent = self.insert_folder(key, parts, i)?;
        }
        Ok(parent)
    }

    /// Records the folder `parts[..=i]` under `key` and returns its number. The same folder
    /// written with other letter case or another Unicode normalization is refused.
    fn insert_folder<S: AsRef<str>>(&mut self, key: (usize, String), parts: &[S], i: usize) -> Result<usize, String> {
        let part = parts.get(i).map_or("", AsRef::as_ref);
        let number = self.folders.len() + 1;
        match self.folders.entry(key) {
            Entry::Occupied(seen) => {
                let (number, first) = seen.get();
                if first == part {
                    return Ok(*number);
                }
                let above = join(parts.get(..i).unwrap_or_default());
                let seen = if above.is_empty() { first.clone() } else { format!("{above}/{first}") };
                let exact = join(parts.get(..=i).unwrap_or_default());
                Err(format!("the folder {exact:?} is also written {seen:?} (names are compared ignoring case and Unicode normalization)"))
            }
            Entry::Vacant(slot) => {
                slot.insert((number, part.to_string()));
                Ok(number)
            }
        }
    }

    /// Bytes of name text the registry holds.
    #[cfg(test)]
    fn kept_bytes(&self) -> usize {
        self.files.iter().map(|(_, k)| k.len()).sum::<usize>() + self.folders.iter().map(|((_, k), (_, v))| k.len() + v.len()).sum::<usize>()
    }
}

fn join<S: AsRef<str>>(parts: &[S]) -> String {
    parts.iter().map(AsRef::as_ref).collect::<Vec<_>>().join("/")
}

/// The key under which [`Names`] compares a part: the lowercase of the uppercase of its lowercase.
/// The forms of one letter then get one key even when one is not the lowercase of another (`Σ`,
/// `σ` and `ς`). A letter whose uppercase is longer gets the key of that spelling (`ß` and `ss`),
/// and the first lowercase gives a capital the key of its small letter (`ẞ`, whose lowercase is
/// `ß`), so such names are refused even on a file system that keeps them apart. The part is put
/// in Unicode Normalization Form C before the case mapping and the key again after it, so a
/// composed `é` and an `e` followed by a combining acute accent, which are one name on macOS, get
/// one key, and so do `α` followed by U+0345 and U+0301 in either order.
fn fold(part: &str) -> String {
    let composed: String = part.nfc().collect();
    composed.to_lowercase().to_uppercase().to_lowercase().nfc().collect()
}

/// Creates `dest` when it is missing and checks that it is a folder and not a link.
fn prepare(dest: &Path) -> Result<(), FetchError> {
    let checked = without_trailing(dest);
    match fs::symlink_metadata(&checked) {
        Ok(m) if m.is_dir() => return Ok(()),
        Ok(_) => return Err(FetchError::NotRegular(dest.to_path_buf())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_error(dest, e)),
    }
    fs::create_dir_all(dest).map_err(|e| io_error(dest, e))?;
    match fs::symlink_metadata(&checked) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(FetchError::NotRegular(dest.to_path_buf())),
        Err(e) => Err(io_error(dest, e)),
    }
}

/// `path` without trailing separators and `.` parts, which make the system follow a link at the
/// last part. A root such as `/` or `C:\` is kept.
fn without_trailing(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Checks that `path` is a folder and not a link. When it is missing, creates it if `create` is
/// set and otherwise returns `Ok(false)`.
fn ensure_dir(path: &Path, create: bool) -> Result<bool, FetchError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => Ok(true),
        Ok(_) => Err(FetchError::NotRegular(path.to_path_buf())),
        Err(e) if e.kind() == io::ErrorKind::NotFound && create => match fs::create_dir(path) {
            Ok(()) => Ok(true),
            // Another process made it first: it is fine when it is a folder.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => ensure_dir(path, false),
            Err(e) => Err(io_error(path, e)),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_error(path, e)),
    }
}

/// The path of `parts` under `dest`, with every folder before the last part checked by
/// [`ensure_dir`]. Without `create`, the checks stop at the first missing folder.
fn walk<S: AsRef<str>>(dest: &Path, parts: &[S], create: bool) -> Result<PathBuf, FetchError> {
    let mut path = dest.to_path_buf();
    let mut present = true;
    for (i, part) in parts.iter().enumerate() {
        path.push(part.as_ref());
        if i + 1 < parts.len() && present {
            present = ensure_dir(&path, create)?;
        }
    }
    Ok(path)
}

/// Whether `target` already holds the pinned bytes. Any other regular file there is deleted.
fn cached(target: &Path, item: &Checked<'_>) -> Result<bool, FetchError> {
    let meta = match fs::symlink_metadata(target) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io_error(target, e)),
    };
    if !meta.is_file() {
        return Err(FetchError::NotRegular(target.to_path_buf()));
    }
    if meta.len() <= item.pin.max_bytes && hash_file(target, item.pin.max_bytes)?.is_some_and(|actual| actual == item.sha256) {
        return Ok(true);
    }
    fs::remove_file(target).map_err(|e| io_error(target, e))?;
    Ok(false)
}

/// The lowercase SHA-256 of the file, or `None` when it holds more than `max_bytes` bytes.
fn hash_file(path: &Path, max_bytes: u64) -> Result<Option<String>, FetchError> {
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    let mut hasher = Sha256::new();
    let hashed = io::copy(&mut file.take(max_bytes.saturating_add(1)), &mut hasher).map_err(|e| io_error(path, e))?;
    Ok((hashed <= max_bytes).then(|| hex(&hasher.finalize())))
}

fn download_file(target: &Path, item: &Checked<'_>, transport: &dyn Transport) -> Result<(), FetchError> {
    let mut failure = None;
    let written = craft_atomic::atomic_write_with(target, |file| {
        let mut sink = Sink::new(file, item.pin.max_bytes);
        let got = transport.download(item.pin.url, item.pin.max_bytes, &mut sink);
        match finish(item.pin.name, &item.sha256, sink, got) {
            Ok(_) => Ok(()),
            Err(e) => {
                let message = e.to_string();
                failure = Some(e);
                Err(io::Error::other(message))
            }
        }
    });
    match (failure, written) {
        (Some(e), _) => Err(e),
        (None, Ok(())) => Ok(()),
        (None, Err(e)) => Err(io_error(target, e)),
    }
}

/// A writer that counts and hashes what passes through it, and fails a write that would take the
/// count past `max`.
struct Sink<W> {
    out: W,
    hasher: Sha256,
    written: u64,
    max: u64,
    over: bool,
}

impl<W: Write> Sink<W> {
    fn new(out: W, max: u64) -> Self {
        Self { out, hasher: Sha256::new(), written: 0, max, over: false }
    }
}

impl<W: Write> Write for Sink<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = self.max.saturating_sub(self.written);
        if self.over || buf.len() as u64 > room {
            self.over = true;
            return Err(io::Error::other(format!("the download is over its limit of {} bytes", self.max)));
        }
        let n = self.out.write(buf)?;
        if let Some(done) = buf.get(..n) {
            self.hasher.update(done);
        }
        self.written = self.written.saturating_add(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// Turns a finished transfer into its output, or into the error that rejects it: the limit first,
/// then the transport's error, then the SHA-256.
fn finish<W>(name: &str, sha256: &str, sink: Sink<W>, got: io::Result<()>) -> Result<W, FetchError> {
    if sink.over || matches!(&got, Err(e) if e.kind() == io::ErrorKind::FileTooLarge) {
        return Err(too_large(name, format!("{} bytes", sink.max)));
    }
    got.map_err(|source| FetchError::Download { name: name.to_string(), source })?;
    let actual = hex(&sink.hasher.finalize());
    if actual != sha256 {
        return Err(FetchError::HashMismatch { name: name.to_string(), expected: sha256.to_string(), actual });
    }
    Ok(sink.out)
}

/// One entry of an archive: a file with its bytes, or a folder.
struct Item {
    parts: Vec<String>,
    data: Option<Vec<u8>>,
}

/// A reader that fails once more than `left` bytes would pass through it.
struct Bounded<R> {
    inner: R,
    left: u64,
    over: bool,
}

impl<R: Read> Read for Bounded<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => {
                    self.over = true;
                    Err(io::Error::other("the archive expands past its limit"))
                }
            };
        }
        let want = buf.len().min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = match buf.get_mut(..want) {
            Some(part) => self.inner.read(part)?,
            None => 0,
        };
        self.left = self.left.saturating_sub(n as u64);
        Ok(n)
    }
}

fn read_tar(name: &str, bytes: &[u8], options: &ExtractOptions) -> Result<Vec<Item>, FetchError> {
    let entries = u64::try_from(options.max_entries).unwrap_or(u64::MAX).saturating_add(1);
    let limit = options.max_expanded_bytes.saturating_add(entries.saturating_mul(ENTRY_OVERHEAD));
    let (result, over) = if bytes.starts_with(&[0x1f, 0x8b]) {
        read_to_end(name, Bounded { inner: flate2::read::MultiGzDecoder::new(bytes), left: limit, over: false }, options)
    } else {
        read_to_end(name, Bounded { inner: bytes, left: limit, over: false }, options)
    };
    if over {
        return Err(too_large(name, format!("{limit} bytes of tar data")));
    }
    result
}

/// Reads the entries, then the rest of the stream after the tar end marker, so that the bytes there
/// count against the limit and a gzip checksum and length, which come last, are checked.
fn read_to_end<R: Read>(name: &str, mut reader: Bounded<R>, options: &ExtractOptions) -> (Result<Vec<Item>, FetchError>, bool) {
    let mut result = read_entries(name, &mut reader, options);
    if result.is_ok()
        && let Err(e) = io::copy(&mut reader, &mut io::sink())
    {
        result = Err(FetchError::Archive { name: name.to_string(), reason: e.to_string() });
    }
    (result, reader.over)
}

fn read_entries<R: Read>(name: &str, reader: R, options: &ExtractOptions) -> Result<Vec<Item>, FetchError> {
    let bad = |reason: String| FetchError::Archive { name: name.to_string(), reason };
    let mut archive = tar::Archive::new(reader);
    let mut names = Names::default();
    let mut items = Vec::new();
    let mut count = 0usize;
    let mut expanded = 0u64;
    for entry in archive.entries().map_err(|e| bad(e.to_string()))? {
        let mut entry = entry.map_err(|e| bad(e.to_string()))?;
        count += 1;
        if count > options.max_entries {
            return Err(too_large(name, format!("{} entries", options.max_entries)));
        }
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions() {
            continue;
        }
        let raw = entry.path_bytes().into_owned();
        let shown = String::from_utf8_lossy(&raw).into_owned();
        let folder = kind.is_dir();
        if !folder && !matches!(kind, EntryType::Regular | EntryType::Continuous) {
            return Err(bad(format!("{shown:?} is a link, a device or another special entry")));
        }
        // Every file counts against the limits, including one that `strip_components` leaves out.
        let size = if folder { 0 } else { entry.size() };
        if size > options.max_entry_bytes {
            return Err(too_large(&format!("{name}: {shown}"), format!("{} bytes", options.max_entry_bytes)));
        }
        expanded = expanded.saturating_add(size);
        if expanded > options.max_expanded_bytes {
            return Err(too_large(name, format!("{} bytes of files", options.max_expanded_bytes)));
        }
        let Some(parts) = entry_parts(&raw, options.strip_components, folder).map_err(|reason| bad(format!("{shown:?}: {reason}")))? else {
            continue;
        };
        if folder {
            names.add_folder(&parts).map_err(|reason| bad(format!("{shown:?}: {reason}")))?;
            items.push(Item { parts, data: None });
            continue;
        }
        names.add_file(&parts).map_err(|reason| bad(format!("{shown:?}: {reason}")))?;
        let mut data = Vec::new();
        entry.by_ref().take(size).read_to_end(&mut data).map_err(|e| bad(e.to_string()))?;
        if data.len() as u64 != size {
            return Err(bad(format!("{shown:?} ends before its {size} bytes")));
        }
        items.push(Item { parts, data: Some(data) });
    }
    Ok(items)
}

/// The checked parts of an entry's path after `strip` leading folders, or `None` for an entry
/// that names no file or folder once stripped (such as `./`).
fn entry_parts(raw: &[u8], strip: usize, folder: bool) -> Result<Option<Vec<String>>, String> {
    let text = std::str::from_utf8(raw).map_err(|_| "the path is not UTF-8".to_string())?;
    if text.starts_with('/') {
        return Err("the path is absolute".into());
    }
    // Only a folder entry may end in `/`; for a file it leaves an empty last part, which is refused.
    let trimmed = if folder { text.strip_suffix('/').unwrap_or(text) } else { text };
    let parts = || trimmed.split('/').skip_while(|p| *p == ".");
    if parts().all(str::is_empty) {
        // Only a folder entry may name the root (`.` or `./`); a file there would be dropped.
        return if folder { Ok(None) } else { Err("the path names no file".into()) };
    }
    // The parts are checked and the kept length measured before anything is collected, so a long
    // path allocates nothing before it is refused.
    let mut kept_bytes = 0usize;
    for (i, part) in parts().enumerate() {
        check_part(part)?;
        if i >= strip {
            kept_bytes = kept_bytes.saturating_add(part.len() + 1);
        }
    }
    if kept_bytes > MAX_NAME_BYTES {
        return Err(format!("the path is longer than {MAX_NAME_BYTES} bytes"));
    }
    let kept: Vec<String> = parts().skip(strip).map(str::to_string).collect();
    if kept.is_empty() {
        return Ok(None);
    }
    Ok(Some(kept))
}

/// Checks, before anything is written, that an archive entry can go to its place under `dest`.
fn check_target(dest: &Path, item: &Item) -> Result<(), FetchError> {
    let path = walk(dest, &item.parts, false)?;
    match fs::symlink_metadata(&path) {
        Ok(m) if item.data.is_some() && m.is_file() => Ok(()),
        Ok(m) if item.data.is_none() && m.is_dir() => Ok(()),
        Ok(_) => Err(FetchError::NotRegular(path)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(&path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curl_reads_no_config_and_allows_https_only() {
        let args = curl_args("https://example.com/a.png", 1234);
        assert_eq!(args.first().map(String::as_str), Some("-q"));
        let joined = args.join(" ");
        for expected in
            ["--globoff", "--fail", "--proto =https", "--proto-redir =https", "--max-filesize 1234", "--output -", "--url https://example.com/a.png"]
        {
            assert!(joined.contains(expected), "{expected} is missing from {joined}");
        }
    }

    #[test]
    fn curl_keeps_dot_segments_and_a_limit_it_can_represent() {
        let args = curl_args("https://example.com/a/../b.png", u64::MAX);
        assert!(args.iter().any(|a| a == "--path-as-is"), "{args:?}");
        let at = args.iter().position(|a| a == "--max-filesize").unwrap();
        assert_eq!(args.get(at + 1).map(String::as_str), Some(i64::MAX.to_string().as_str()));
    }

    #[test]
    fn a_cached_file_that_grew_past_the_limit_is_not_hashed_as_a_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, b"0123456789").unwrap();
        assert!(hash_file(&path, 10).unwrap().is_some());
        assert!(hash_file(&path, 9).unwrap().is_none());
    }

    #[test]
    fn only_a_folder_entry_may_end_in_a_slash() {
        assert_eq!(entry_parts(b"top/", 0, true), Ok(Some(vec!["top".to_string()])));
        assert!(entry_parts(b"top/", 0, false).is_err());
        assert_eq!(entry_parts(b"top/a.png", 0, false), Ok(Some(vec!["top".to_string(), "a.png".to_string()])));
    }

    #[test]
    fn entry_parts_keep_their_results_around_the_length_limit() {
        let long = "a/".repeat(MAX_NAME_BYTES);
        assert!(entry_parts(long.as_bytes(), 0, true).is_err());
        // Stripped folders do not count toward the length, but are still checked.
        let fits = format!("{}b", "a/".repeat(MAX_NAME_BYTES / 2));
        let strip = MAX_NAME_BYTES / 2;
        assert_eq!(entry_parts(fits.as_bytes(), strip, false), Ok(Some(vec!["b".to_string()])));
        assert!(entry_parts(format!("..{}", &fits[1..]).as_bytes(), strip, false).is_err());
        assert_eq!(entry_parts(b"./", 0, true), Ok(None));
        assert_eq!(entry_parts(b"top/", 1, true), Ok(None));
    }

    #[test]
    fn sha256_hex_matches_the_fips_180_example() {
        // FIPS 180-4 example "abc".
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn long_names_fit_the_tar_data_bound() {
        let folder = "f".repeat(99);
        let mut builder = tar::Builder::new(Vec::new());
        for i in 0..3 {
            let name = format!("{}/{i}{}", [folder.as_str(); 40].join("/"), "n".repeat(90));
            assert!(name.len() > 4000 && name.len() < MAX_NAME_BYTES);
            let mut header = tar::Header::new_gnu();
            header.set_size(1);
            builder.append_data(&mut header, &name, &b"x"[..]).unwrap();
        }
        let archive = builder.into_inner().unwrap();
        let options = ExtractOptions { max_entries: 3, max_entry_bytes: 1, max_expanded_bytes: 3, strip_components: 0 };
        let items = read_tar("t.tar", &archive, &options).unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn the_destination_is_checked_without_trailing_separators() {
        assert_eq!(without_trailing(Path::new("cache/")), Path::new("cache"));
        assert_eq!(without_trailing(Path::new("a/cache//.")), Path::new("a/cache"));
        assert_eq!(without_trailing(Path::new("/")), Path::new("/"));
        assert_eq!(without_trailing(Path::new(".")), Path::new("."));
        #[cfg(windows)]
        {
            assert_eq!(without_trailing(Path::new(r"C:\cache\")).as_os_str(), r"C:\cache");
            assert_eq!(without_trailing(Path::new(r"C:\")).as_os_str(), r"C:\");
        }
    }

    #[test]
    fn the_name_registry_grows_with_the_names_and_not_their_depth() {
        // Deep paths with a different first folder each share no folder, so a registry that keeps
        // every folder's whole path would hold text that grows with the square of the depth.
        let mut names = Names::default();
        let mut given = 0;
        for i in 0..50 {
            let mut parts = vec![format!("d{i:04}")];
            parts.extend(std::iter::repeat_n("a".to_string(), 400));
            parts.push("f".to_string());
            given += join(&parts).len();
            names.add_file(&parts).unwrap();
        }
        assert!(names.kept_bytes() <= 4 * given, "{} bytes kept for {given} bytes of names", names.kept_bytes());
    }

    #[test]
    fn a_fold_key_is_at_most_three_times_its_part() {
        for c in (0..=u32::from(char::MAX)).filter_map(char::from_u32) {
            let key = fold(c.encode_utf8(&mut [0; 4]));
            assert!(key.len() <= 3 * c.len_utf8(), "{c:?} folds to {key:?}");
        }
        assert_eq!(fold("\u{3a3}"), fold("\u{3c2}"));
        assert_ne!(fold("a.txt"), fold("b.txt"));
    }

    #[test]
    fn the_capital_and_small_sharp_s_are_one_name() {
        assert_eq!(fold("\u{1e9e}"), fold("\u{df}"));
        assert_eq!(fold("\u{df}"), fold("ss"));
        let mut names = Names::default();
        names.add_file(&["stra\u{df}e.png"]).unwrap();
        assert!(names.add_file(&["STRA\u{1e9e}E.png"]).is_err());
        let mut folders = Names::default();
        folders.add_folder(&["\u{1e9e}"]).unwrap();
        assert!(folders.add_folder(&["\u{df}"]).is_err());
    }

    #[test]
    fn a_part_is_at_most_255_bytes() {
        assert!(check_part(&"a".repeat(MAX_PART_BYTES)).is_ok());
        assert!(check_part(&"a".repeat(MAX_PART_BYTES + 1)).is_err());
        assert!(split_name(&format!("top/{}", "\u{e9}".repeat(128))).is_err());
        assert!(entry_parts(format!("top/{}", "b".repeat(MAX_PART_BYTES + 1)).as_bytes(), 0, false).is_err());
    }

    #[test]
    fn only_a_folder_entry_may_name_the_root() {
        assert_eq!(entry_parts(b".", 0, true), Ok(None));
        assert_eq!(entry_parts(b"./", 0, true), Ok(None));
        assert!(entry_parts(b".", 0, false).is_err());
        assert!(entry_parts(b"./.", 0, false).is_err());
    }

    #[test]
    fn messages_escape_control_characters_in_names() {
        assert_eq!(invalid("a\nb\u{1b}", "refused".into()).to_string(), "a\\nb\\u{1b}: refused");
        assert_eq!(too_large("a\rb", "1 bytes".into()).to_string(), "a\\rb is over its limit of 1 bytes");
        let archive = FetchError::Archive { name: "t\u{7}.tar".into(), reason: "bad".into() };
        assert_eq!(archive.to_string(), "t\\u{7}.tar: bad");
    }

    #[test]
    fn bounded_reader_fails_only_past_its_limit() {
        let mut exact = Bounded { inner: &b"abcd"[..], left: 4, over: false };
        let mut out = Vec::new();
        exact.read_to_end(&mut out).unwrap();
        assert_eq!((out.as_slice(), exact.over), (&b"abcd"[..], false));
        let mut short = Bounded { inner: &b"abcde"[..], left: 4, over: false };
        assert!(short.read_to_end(&mut Vec::new()).is_err());
        assert!(short.over);
    }
}
