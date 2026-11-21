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
//! and other special entries, absolute paths, `..`, names that differ only in case, and names that
//! Windows cannot hold are refused.
//!
//! The pinned list and the destination come from the caller. Only `https://` addresses are
//! accepted. [`Curl`] runs the system `curl`, limited to HTTPS for the address and every redirect;
//! a [`Transport`] supplied by the caller replaces it, for example in tests.
//!
//! Neither the destination nor any path under it may be a link: a link where the destination, a
//! folder or a file goes is refused with [`FetchError::NotRegular`]. The folders above the
//! destination are the caller's path and are followed like any other path, links included (on
//! macOS the system temporary folder is under the link `/var`).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};
use tar::EntryType;

/// Default for [`ExtractOptions::max_entries`].
pub const DEFAULT_MAX_ENTRIES: usize = 1000;
/// Default for [`ExtractOptions::max_entry_bytes`]: 4 MiB.
pub const DEFAULT_MAX_ENTRY_BYTES: u64 = 4 << 20;
/// Default for [`ExtractOptions::max_expanded_bytes`]: 16 MiB.
pub const DEFAULT_MAX_EXPANDED_BYTES: u64 = 16 << 20;

/// Longest name accepted, in bytes.
const MAX_NAME_BYTES: usize = 4096;
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
            FetchError::Invalid { name, reason } => write!(f, "{name}: {reason}"),
            FetchError::NotRegular(path) => write!(f, "{} is a link or another kind of file than expected", path.display()),
            FetchError::TooLarge { name, limit } => write!(f, "{name} is over its limit of {limit}"),
            FetchError::HashMismatch { name, expected, actual } => write!(f, "{name}: SHA-256 is {actual}, but the pin is {expected}"),
            FetchError::Download { name, source } => write!(f, "downloading {name} failed: {source}"),
            FetchError::Archive { name, reason } => write!(f, "{name}: {reason}"),
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
    /// counts the bytes itself and stops the transfer with a write error past it.
    fn download(&self, url: &str, max_bytes: u64, out: &mut dyn Write) -> io::Result<()>;
}

/// The [`Transport`] of [`fetch_pinned`] and [`fetch_archive`]: the system `curl`, reading no
/// configuration file, allowed HTTPS only for the address and every redirect, failing on an HTTP
/// error status, and stopping a transfer that stays under 1 KiB per second for 60 seconds.
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
        if status.success() { Ok(()) } else { Err(io::Error::other(format!("curl {url} failed: {status}"))) }
    }
}

/// The arguments [`Curl`] passes: `-q` first, so no `.curlrc` changes them.
fn curl_args(url: &str, max_bytes: u64) -> Vec<String> {
    let mut args: Vec<String> = [
        "-q",
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
    args.push(max_bytes.to_string());
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
/// portable parts that no other pin repeats (ignoring case) or uses as a folder, its address
/// starts with `https://`, its SHA-256 has 64 hexadecimal digits and its `max_bytes` is not 0.
/// `dest` and the folders the names need are created. The files are then handled in order, and
/// the first failure stops the fetch; files handled before it stay in place.
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
/// writes its files under `dest` as [`extract`] does. The archive is held in memory, so
/// `archive.max_bytes` also bounds the memory it takes. Returns the names of the files written.
pub fn fetch_archive_with(dest: &Path, archive: &Pin<'_>, options: &ExtractOptions, transport: &dyn Transport) -> Result<Vec<String>, FetchError> {
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
/// holds `..`, is not UTF-8 or is not portable, a file name repeated ignoring case, and a name
/// used for both a file and a folder each stop it with nothing written. Each file is then written
/// through a temporary file and a rename; an existing file at that name is replaced.
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

/// File and folder names seen so far, keyed in lowercase, so that names differing only in case,
/// which are one file or folder on macOS and Windows, are refused.
#[derive(Default)]
struct Names {
    files: HashSet<String>,
    /// Each folder's lowercase path, mapped to the path as first written.
    folders: HashMap<String, String>,
}

impl Names {
    fn add_file<S: AsRef<str>>(&mut self, parts: &[S]) -> Result<(), String> {
        self.add_parents(parts)?;
        let key = lower_join(parts);
        if self.folders.contains_key(&key) {
            return Err("the name is also used as a folder".into());
        }
        if !self.files.insert(key) {
            return Err("the name appears twice (names are compared ignoring case)".into());
        }
        Ok(())
    }

    fn add_folder<S: AsRef<str>>(&mut self, parts: &[S]) -> Result<(), String> {
        self.add_parents(parts)?;
        let key = lower_join(parts);
        if self.files.contains(&key) {
            return Err("the name is also used as a file".into());
        }
        self.insert_folder(key, join(parts))
    }

    fn add_parents<S: AsRef<str>>(&mut self, parts: &[S]) -> Result<(), String> {
        let folders = parts.len().saturating_sub(1);
        for end in 1..=folders {
            let prefix = parts.get(..end).unwrap_or_default();
            let key = lower_join(prefix);
            if self.files.contains(&key) {
                return Err(format!("the name is inside {key:?}, which is also a file"));
            }
            self.insert_folder(key, join(prefix))?;
        }
        Ok(())
    }

    /// Records a folder; the same folder written with other letter case is refused.
    fn insert_folder(&mut self, key: String, exact: String) -> Result<(), String> {
        match self.folders.get(&key) {
            Some(seen) if *seen != exact => Err(format!("the folder {exact:?} is also written {seen:?} (names are compared ignoring case)")),
            Some(_) => Ok(()),
            None => {
                self.folders.insert(key, exact);
                Ok(())
            }
        }
    }
}

fn join<S: AsRef<str>>(parts: &[S]) -> String {
    parts.iter().map(AsRef::as_ref).collect::<Vec<_>>().join("/")
}

fn lower_join<S: AsRef<str>>(parts: &[S]) -> String {
    join(parts).to_lowercase()
}

/// Creates `dest` when it is missing and checks that it is a folder and not a link.
fn prepare(dest: &Path) -> Result<(), FetchError> {
    fs::create_dir_all(dest).map_err(|e| io_error(dest, e))?;
    match fs::symlink_metadata(dest) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(FetchError::NotRegular(dest.to_path_buf())),
        Err(e) => Err(io_error(dest, e)),
    }
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
    if meta.len() <= item.pin.max_bytes && hash_file(target, item.pin.max_bytes)? == item.sha256 {
        return Ok(true);
    }
    fs::remove_file(target).map_err(|e| io_error(target, e))?;
    Ok(false)
}

fn hash_file(path: &Path, max_bytes: u64) -> Result<String, FetchError> {
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file.take(max_bytes.saturating_add(1)), &mut hasher).map_err(|e| io_error(path, e))?;
    Ok(hex(&hasher.finalize()))
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
    if sink.over {
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
        let Some(parts) = entry_parts(&raw, options.strip_components).map_err(|reason| bad(format!("{shown:?}: {reason}")))? else {
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
fn entry_parts(raw: &[u8], strip: usize) -> Result<Option<Vec<String>>, String> {
    let text = std::str::from_utf8(raw).map_err(|_| "the path is not UTF-8".to_string())?;
    if text.starts_with('/') {
        return Err("the path is absolute".into());
    }
    let parts: Vec<&str> = text.strip_suffix('/').unwrap_or(text).split('/').skip_while(|p| *p == ".").collect();
    if parts.iter().all(|p| p.is_empty()) {
        return Ok(None);
    }
    for part in &parts {
        check_part(part)?;
    }
    let kept: Vec<String> = parts.iter().skip(strip).map(|p| (*p).to_string()).collect();
    if kept.is_empty() {
        return Ok(None);
    }
    if kept.iter().map(|p| p.len() + 1).sum::<usize>() > MAX_NAME_BYTES {
        return Err(format!("the path is longer than {MAX_NAME_BYTES} bytes"));
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
        for expected in ["--fail", "--proto =https", "--proto-redir =https", "--max-filesize 1234", "--output -", "--url https://example.com/a.png"] {
            assert!(joined.contains(expected), "{expected} is missing from {joined}");
        }
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
