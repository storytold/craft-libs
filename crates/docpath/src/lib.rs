#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Decide whether a file path or file URL stored in a document may be opened, read or written.
//!
//! Paths stored in documents (linked images, templates, data sources, media) are only followed when
//! they refer to a local file, or to a file on the same network share the document itself was opened
//! from. Paths to other computers, device paths and non-file URLs are refused, so opening a document
//! never makes the app contact a server or device the user did not choose.
//!
//! The check is lexical: it does no I/O and, apart from colons, gives the same answer on every
//! platform. Windows path forms (`C:\`, `\\server\share`, `\\?\`, `\\.\`, reserved names such as
//! `NUL`) are recognised on all platforms, and both `/` and `\` count as separators.
//!
//! - [`classify`] sorts a stored path into a [`PathKind`].
//! - [`is_local`] is `classify(raw) == PathKind::Local`.
//! - [`resolve_absolute`] turns a stored path into the absolute path to open, relative to the
//!   document, or a [`Refused`] that says why it may not be opened. Use it to open files.
//! - [`resolve`] is the same without the absolute check, for callers that only display or compare
//!   the result.
//!
//! # Colons
//!
//! A colon after a drive letter at the start (`C:\`, `C:/`, `C:x`, `\\?\C:\`, `file:///C:/`) names
//! a drive on every platform. Other colons depend on the platform the check runs on:
//!
//! | Stored path | Windows | macOS and Linux |
//! |---|---|---|
//! | URL-shaped (see below): `https://h/x`, `mailto:a@b`, `jar:…`, `view-source:…` | [`PathKind::Url`] | [`PathKind::Url`] |
//! | Starts with any other scheme and a colon: `chapter1:intro.png`, `notes:v2.pdf` | [`PathKind::Url`] | allowed |
//! | A colon inside any other name: `2024:05/a.jpg`, `/Users/a/2024:05/x.jpg` | [`PathKind::Invalid`] | allowed |
//!
//! Windows reads a colon inside a name as an alternate data stream (`a.png:x` is the stream `x` of
//! `a.png`) and hands `name:…` to URL protocol handlers, so it refuses both. On macOS and Linux a
//! colon is an ordinary character: Finder shows a `/` in a file or folder name as `:` in the POSIX
//! path, so a Lightroom folder named `2024/05` is stored as `2024:05`.
//!
//! A stored path is URL-shaped when it starts with an RFC 3986 scheme of two or more characters and
//! a colon, and either
//!
//! - its first component (the text before the first `/` or `\`) ends with a colon, as in
//!   `https://h`, `http:\\h`, `shell:` or the nested `view-source:http://h` and
//!   `jar:file:///x.jar!/a`, or
//! - the scheme, or one of its `+`-separated parts (`svn+ssh`), compared ignoring case, starts with
//!   `ms-` (the prefix of Windows app protocols) or is one of `about`, `blob`, `data`, `file`,
//!   `ftp`, `ftps`, `http`, `https`, `jar`, `javascript`, `mailto`, `news`, `nntp`, `search-ms`,
//!   `sftp`, `shell`, `smb`, `sms`, `ssh`, `tel`, `vbscript`, `view-source`, `webdav`, `ws` or `wss`.
//!
//! Names of files and folders rarely end their first component with a colon or start with one of
//! these scheme names, so names such as `Shoot: Day 1`, `2024:05` and `notes:v2.pdf` stay usable,
//! while links that browsers or the desktop would open as URLs are refused. A folder whose name
//! starts with such a word and a slash (`Data/2024`, stored as `Data:2024`) is refused too.
//!
//! # Caller contract
//!
//! - Pass the document whenever there is one. Without it, a relative, root-relative or
//!   drive-relative result depends on the process working directory, which can be a network share;
//!   [`resolve_absolute`] refuses such a result.
//! - Hand the result to the filesystem through Rust `std` or wide (`W`) Windows APIs, and do not
//!   percent-decode or normalise it again: an ANSI API or a second decoding pass can turn the
//!   checked path into a different one.
//! - On Windows, when the document was opened from a mapped drive, pass its UNC path instead if the
//!   app can get it (`WNetGetConnectionW`), so links on the same share resolve as that share.
//! - A lexical check cannot see mapped drives, `subst` drives, symlinks or junctions: a local-looking
//!   path can still lead to another computer through them.
//!
//! ```
//! use std::path::Path;
//! use craft_docpath::{resolve, resolve_absolute, Refused};
//!
//! let doc = Path::new(r"\\server\share\docs\report.doc");
//! assert!(resolve(r"img\logo.png", Some(doc)).is_ok());
//! assert!(resolve(r"\\server\share\img\logo.png", Some(doc)).is_ok());
//! assert_eq!(resolve(r"\\other\share\logo.png", Some(doc)), Err(Refused::Network));
//! assert_eq!(resolve_absolute("img/logo.png", None), Err(Refused::Invalid));
//! ```

use std::fmt;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

/// What a stored path refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathKind {
    /// A file on this computer: absolute, drive-relative, root-relative or relative.
    Local,
    /// A file on another computer: `\\host\share`, `//host/share`, `\\?\UNC\…`, `file://host/…`.
    Network,
    /// A device or object-manager path: `\\.\…`, `\??\…`, `\\?\GLOBALROOT\…`, or a reserved name such as `NUL` or `COM1`.
    Device,
    /// A URL other than `file:`, such as `https://…`, `mailto:`, `jar:…` or `view-source:…`. On
    /// Windows, also anything else that starts with a URL scheme of two or more characters and a
    /// colon, such as `chapter1:intro.png`. See [Colons](crate#colons).
    Url,
    /// Not a usable path: empty, containing control characters or lookalike separators, a name made
    /// of dots, or climbing above its root with `..`. On Windows, also a colon inside a name
    /// (`img/fig:1.png`, `C:\x:stream`); see [Colons](crate#colons).
    Invalid,
}

/// Why [`resolve`] refused a stored path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refused {
    /// The path is on another computer, or on a network share other than the document's.
    Network,
    /// The path names a device rather than a file.
    Device,
    /// The link is a URL, not a file path.
    Url,
    /// The path is not usable.
    Invalid,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Network => "the path is on another computer",
            Self::Device => "the path names a device, not a file",
            Self::Url => "the link is not a file path",
            Self::Invalid => "the path is not valid",
        })
    }
}

impl std::error::Error for Refused {}

impl From<Refused> for PathKind {
    fn from(r: Refused) -> Self {
        match r {
            Refused::Network => Self::Network,
            Refused::Device => Self::Device,
            Refused::Url => Self::Url,
            Refused::Invalid => Self::Invalid,
        }
    }
}

/// Classify a path or URL stored in a document, without a document to resolve it against.
///
/// Relative paths are [`PathKind::Local`]; a rooted path whose `..` components climb above its
/// drive, share or `/` is [`PathKind::Invalid`].
pub fn classify(raw: &str) -> PathKind {
    match resolve(raw, None) {
        Ok(_) => PathKind::Local,
        Err(r) => r.into(),
    }
}

/// Whether a path or URL stored in a document refers to a local file (see [`classify`]).
pub fn is_local(raw: &str) -> bool {
    classify(raw) == PathKind::Local
}

/// Turn a path or URL stored in a document into the absolute path to open, or refuse it. This is
/// the function to call before opening, reading or writing the file.
///
/// It is [`resolve`] followed by a check that the result is absolute on the running platform
/// ([`Path::is_absolute`]); a result that is not, such as a relative link in a document that has
/// no path yet, is refused as [`Refused::Invalid`]. On Unix, Windows-form results (`C:\x`,
/// `\\server\share\x`) are not absolute and are refused too.
pub fn resolve_absolute(raw: &str, document: Option<&Path>) -> Result<PathBuf, Refused> {
    let path = resolve(raw, document)?;
    if path.is_absolute() { Ok(path) } else { Err(Refused::Invalid) }
}

/// Turn a path or URL stored in a document into the path to open, or refuse it. The result may be
/// relative; to open a file, use [`resolve_absolute`].
///
/// `document` is the path of the document the link came from, if it has one. The steps:
///
/// 1. A `file:` URL is percent-decoded (as Windows-1252 where the escapes are not UTF-8) and its
///    scheme removed. Only an empty host, `localhost` or a drive (`file://C:/x`) is local;
///    `file://host/…` is refused as [`Refused::Network`], and a URL with no path as
///    [`Refused::Invalid`].
/// 2. A relative path is joined to the document's folder, a root-relative path (`\x`) to the
///    document's drive or share, and a drive-relative path (`C:x`) to the document's folder when the
///    document is on that drive. Without a document they are returned unresolved.
/// 3. `.` and `..` are folded lexically; climbing above a drive, share or `/` is [`Refused::Invalid`].
/// 4. A network path is allowed only when it is on the document's own share (same `\\server\share`,
///    compared ignoring ASCII case). Other network paths, device paths and reserved device names
///    are refused.
///
/// Windows-form results (drive, share, `\\?\`) use `\` as the separator on every platform; others
/// use the platform separator.
pub fn resolve(raw: &str, document: Option<&Path>) -> Result<PathBuf, Refused> {
    let plain = strip_url(raw)?;
    let link = parse(&plain, true)?;

    let doc_str = document.and_then(Path::to_str);
    if let (Some(doc), None, Root::Relative) = (document, doc_str, &link.root) {
        return join_native(doc, &link.comps);
    }
    let doc = doc_str.map(|d| parse(d, false));
    let doc_share = match &doc {
        Some(Ok(Parsed { root: Root::Share { host, share, .. }, .. })) => Some((host.clone(), share.clone())),
        _ => None,
    };

    let (root, comps) = match (link.root, doc) {
        (Root::Relative, Some(doc)) => {
            let doc = doc?;
            (doc.root, folder(doc.comps).chain(link.comps).collect())
        }
        (Root::Rooted, Some(Ok(doc))) if doc.root.is_absolute() => (doc.root, link.comps),
        (Root::DriveRelative(l), Some(Ok(doc))) if doc.root.drive().is_some_and(|d| d.eq_ignore_ascii_case(&l)) => {
            (doc.root, folder(doc.comps).chain(link.comps).collect())
        }
        (root, _) => (root, link.comps),
    };
    let comps = normalize(&root, comps)?;

    if let Root::Share { host, share, .. } = &root {
        let same = doc_share.is_some_and(|(h, s)| h.eq_ignore_ascii_case(host) && s.eq_ignore_ascii_case(share));
        if !same {
            return Err(Refused::Network);
        }
    }
    Ok(render(&root, &comps))
}

#[derive(Debug)]
enum Root {
    /// `a\b`
    Relative,
    /// `\a` or `/a`
    Rooted,
    /// `C:a`
    DriveRelative(char),
    /// `C:\a`
    Drive(char),
    /// `\\?\C:\a`
    VerbatimDrive(char),
    /// `\\host\share\a`, or `\\?\UNC\host\share\a` when `verbatim`
    Share { host: String, share: String, verbatim: bool },
}

impl Root {
    fn is_absolute(&self) -> bool {
        !matches!(self, Self::Relative | Self::DriveRelative(_))
    }

    fn drive(&self) -> Option<char> {
        match self {
            Self::Drive(l) | Self::VerbatimDrive(l) => Some(*l),
            _ => None,
        }
    }
}

struct Parsed {
    root: Root,
    comps: Vec<String>,
}

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// The document's folder: its components without the file name.
fn folder(mut comps: Vec<String>) -> impl Iterator<Item = String> {
    comps.pop();
    comps.into_iter()
}

/// Reject control characters and non-file URLs; reduce a `file:` URL to its path.
fn strip_url(raw: &str) -> Result<String, Refused> {
    if raw.is_empty() || raw.chars().any(is_bad_char) {
        return Err(Refused::Invalid);
    }
    let path = match raw.get(..5) {
        Some(scheme) if scheme.eq_ignore_ascii_case("file:") => file_url_path(raw.get(5..).unwrap_or_default())?,
        _ => raw.to_string(),
    };
    if has_scheme(&path) {
        // `NUL:` and `COM1:` name devices, not URL schemes.
        if path.split_once(':').is_some_and(|(scheme, _)| is_dos_device(scheme)) {
            return Err(Refused::Device);
        }
        if COLON_IN_NAME_REFUSED || is_url_shaped(&path) {
            return Err(Refused::Url);
        }
    }
    Ok(path)
}

/// Windows reads a colon inside a name as an alternate data stream and `name:…` as a URL for a
/// protocol handler; macOS and Linux allow colons in names (see [Colons](crate#colons)).
const COLON_IN_NAME_REFUSED: bool = cfg!(windows);

/// URL schemes refused on every platform even when the rest does not look like a URL, compared
/// ignoring ASCII case. Any scheme starting with `ms-` is refused too. The module docs list them.
const URL_SCHEMES: &[&str] = &[
    "about",
    "blob",
    "data",
    "file",
    "ftp",
    "ftps",
    "http",
    "https",
    "jar",
    "javascript",
    "mailto",
    "news",
    "nntp",
    "search-ms",
    "sftp",
    "shell",
    "smb",
    "sms",
    "ssh",
    "tel",
    "vbscript",
    "view-source",
    "webdav",
    "ws",
    "wss",
];

/// A stored path that starts with a scheme ([`has_scheme`]) and reads as a URL on every platform:
/// its first component ends with a colon (`https://h`, `http:\\h`, `shell:`,
/// `view-source:http://h`), or the scheme, or one of its `+`-separated parts, is in
/// [`URL_SCHEMES`] or starts with `ms-`.
fn is_url_shaped(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else { return false };
    let first = s.split(is_sep).next().unwrap_or_default();
    first.ends_with(':')
        || scheme.split('+').any(|part| {
            let part = part.to_ascii_lowercase();
            part.starts_with("ms-") || URL_SCHEMES.contains(&part.as_str())
        })
}

/// Control characters, and separator lookalikes that some code page best-fit mappings turn into
/// `\` or `/` when a path reaches an ANSI API.
fn is_bad_char(c: char) -> bool {
    c.is_control() || matches!(c, '\u{FF3C}' | '\u{FF0F}' | '\u{2215}' | '\u{29F5}')
}

/// Starts with a URL scheme (RFC 3986: a letter, then letters, digits, `+`, `-` or `.`) and a
/// colon. The scheme has two or more characters, so a drive letter (`C:`) is not one.
fn has_scheme(s: &str) -> bool {
    let Some((scheme, _)) = s.split_once(':') else { return false };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && scheme.len() >= 2
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The path of a `file:` URL (the part after `file:`).
fn file_url_path(rest: &str) -> Result<String, Refused> {
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let decoded = percent_decode(rest);
    if decoded.is_empty() || decoded.chars().any(is_bad_char) {
        return Err(Refused::Invalid);
    }
    let mut chars = decoded.chars();
    let path = match (chars.next(), chars.next()) {
        (Some(a), Some(b)) if is_sep(a) && is_sep(b) => {
            let after = chars.as_str();
            let (host, path) = match after.find(is_sep) {
                Some(i) => after.split_at(i),
                None => (after, ""),
            };
            // `file://C:/x` and `file://C|/x` name the drive path `C:/x`.
            let mut h = host.chars();
            if let (Some(l), Some(':' | '|'), None) = (h.next(), h.next(), h.next())
                && l.is_ascii_alphabetic()
            {
                return Ok(format!("{l}:{path}"));
            }
            if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
                return Err(Refused::Network);
            }
            if path.is_empty() {
                return Err(Refused::Invalid);
            }
            path
        }
        _ => decoded.as_str(),
    };
    // `/C:/x` and `/C|/x` name the drive path `C:/x`.
    let mut chars = path.chars();
    if let (Some(s), Some(l), Some(':' | '|')) = (chars.next(), chars.next(), chars.next())
        && is_sep(s)
        && l.is_ascii_alphabetic()
        && chars.clone().next().is_none_or(is_sep)
    {
        return Ok(format!("{l}:{}", chars.as_str()));
    }
    Ok(path.to_string())
}

/// Percent-decode a `file:` URL path. A run of escapes that is not UTF-8 is read as Windows-1252,
/// which older Windows software used in file URLs; it maps 0x2F and 0x5C to `/` and `\` as UTF-8 does.
fn percent_decode(s: &str) -> String {
    let hex = |c: Option<&u8>| c.and_then(|&c| (c as char).to_digit(16)).and_then(|d| u8::try_from(d).ok());
    let mut out = String::with_capacity(s.len());
    let mut run: Vec<u8> = Vec::new();
    let mut rest = s;
    loop {
        let bytes = rest.as_bytes();
        if let (Some(b'%'), Some(h), Some(l)) = (bytes.first(), hex(bytes.get(1)), hex(bytes.get(2))) {
            run.push((h << 4) | l);
            rest = rest.get(3..).unwrap_or_default();
            continue;
        }
        match std::str::from_utf8(&run) {
            Ok(text) => out.push_str(text),
            Err(_) => out.extend(run.iter().map(|&b| windows_1252(b))),
        }
        run.clear();
        let mut chars = rest.chars();
        let Some(c) = chars.next() else { break };
        out.push(c);
        rest = chars.as_str();
    }
    out
}

/// Bytes 0x81, 0x8D, 0x8F, 0x90 and 0x9D are unassigned and map to C1 controls, which are refused.
fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or(char::REPLACEMENT_CHARACTER),
        _ => char::from(b),
    }
}

/// Split a plain path into its root and components. `check` applies the component rules
/// (reserved device names, dot-and-space names, and colons on Windows); the document's own path is
/// not checked.
fn parse(s: &str, check: bool) -> Result<Parsed, Refused> {
    let mut chars = s.chars();
    let first = chars.next();
    let second = chars.next();
    let rest = chars.as_str();
    match (first, second) {
        (Some(a), Some(b)) if is_sep(a) && is_sep(b) => parse_double(s, check),
        (Some(a), _) if is_sep(a) => {
            let after = s.get(1..).unwrap_or_default();
            if let Some(r) = after.strip_prefix("??")
                && r.chars().next().is_none_or(is_sep)
            {
                return Err(Refused::Device);
            }
            Ok(Parsed { root: Root::Rooted, comps: comps(after, check)? })
        }
        (Some(l), Some(':')) if l.is_ascii_alphabetic() => {
            let mut r = rest.chars();
            match r.next() {
                Some(c) if is_sep(c) => Ok(Parsed { root: Root::Drive(l), comps: comps(r.as_str(), check)? }),
                _ => Ok(Parsed { root: Root::DriveRelative(l), comps: comps(rest, check)? }),
            }
        }
        _ => Ok(Parsed { root: Root::Relative, comps: comps(s, check)? }),
    }
}

/// A path starting with two separators: a share, or a `\\?\`, `\\.\` or `//?/` prefix.
fn parse_double(s: &str, check: bool) -> Result<Parsed, Refused> {
    let t = s.get(2..).unwrap_or_default();
    let (first, after) = t.split_once(is_sep).unwrap_or((t, ""));
    if first != "?" && first != "." {
        return parse_share(t, false, check);
    }
    if let Some(v) = s.strip_prefix(r"\\?\") {
        return parse_verbatim(v, check);
    }
    // `\\.\`, and `\\?\` spelled with any `/`, which Windows normalises into the device namespace.
    let (next, rest) = after.split_once(is_sep).unwrap_or((after, ""));
    if next.eq_ignore_ascii_case("UNC") {
        return parse_share(rest, false, check);
    }
    Err(Refused::Device)
}

/// `\\?\` paths: only `\`, and no normalisation, so `.`, `..` and `/` are not usable in them.
fn parse_verbatim(v: &str, check: bool) -> Result<Parsed, Refused> {
    let (first, rest) = match v.split_once('\\') {
        Some((f, r)) => (f, Some(r)),
        None => (v, None),
    };
    let mut drive = first.chars();
    match (drive.next(), drive.next(), drive.next(), rest) {
        // `\\?\C:` without a separator is the volume, not its root folder.
        (Some(l), Some(':'), None, Some(rest)) if l.is_ascii_alphabetic() => {
            Ok(Parsed { root: Root::VerbatimDrive(l), comps: verbatim_comps(rest, check)? })
        }
        _ if first.eq_ignore_ascii_case("UNC") && rest.is_some() => parse_share(rest.unwrap_or_default(), true, check),
        _ => Err(Refused::Device),
    }
}

/// `host\share\rest`, the part after the leading separators (or `\\?\UNC\`).
fn parse_share(t: &str, verbatim: bool, check: bool) -> Result<Parsed, Refused> {
    let split = |s: &str| -> (String, String) {
        let pair = if verbatim { s.split_once('\\') } else { s.split_once(is_sep) };
        let (a, b) = pair.unwrap_or((s, ""));
        (a.to_string(), b.to_string())
    };
    let (host, rest) = split(t);
    let (share, rest) = split(&rest);
    if host.is_empty() || share.is_empty() {
        return Err(Refused::Network);
    }
    let comps = if verbatim { verbatim_comps(&rest, check)? } else { comps(&rest, check)? };
    Ok(Parsed { root: Root::Share { host, share, verbatim }, comps })
}

fn comps(s: &str, check: bool) -> Result<Vec<String>, Refused> {
    s.split(is_sep)
        .filter(|c| !c.is_empty())
        .map(|c| {
            if check {
                if is_dos_device(c) {
                    return Err(Refused::Device);
                }
                if is_dot_name(c) || has_refused_colon(c) {
                    return Err(Refused::Invalid);
                }
            }
            Ok(c.to_string())
        })
        .collect()
}

fn verbatim_comps(s: &str, check: bool) -> Result<Vec<String>, Refused> {
    let parts: Vec<&str> = s.split('\\').collect();
    let last = parts.len().saturating_sub(1);
    let mut out = Vec::with_capacity(parts.len());
    for (i, c) in parts.into_iter().enumerate() {
        if c.is_empty() && i == last {
            continue;
        }
        if c.is_empty() || c == "." || c == ".." || c.contains('/') {
            return Err(Refused::Invalid);
        }
        if check && is_dos_device(c) {
            return Err(Refused::Device);
        }
        if check && (is_dot_name(c) || has_refused_colon(c)) {
            return Err(Refused::Invalid);
        }
        out.push(c.to_string());
    }
    Ok(out)
}

/// A colon inside a name, which only Windows refuses (see [Colons](crate#colons)).
fn has_refused_colon(comp: &str) -> bool {
    COLON_IN_NAME_REFUSED && comp.contains(':')
}

/// A name of dots and spaces other than `.` and `..`. Windows trims trailing dots and spaces, so
/// `.. ` or `...` could act as `..`.
fn is_dot_name(comp: &str) -> bool {
    !comp.is_empty() && comp != "." && comp != ".." && comp.chars().all(|ch| ch == '.' || ch == ' ')
}

/// A reserved DOS device name, with or without an extension, stream or trailing dots and spaces.
fn is_dos_device(comp: &str) -> bool {
    let stem = comp.split(['.', ':']).next().unwrap_or_default().trim_end_matches(' ');
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].iter().any(|n| stem.eq_ignore_ascii_case(n)) {
        return true;
    }
    let Some(head) = stem.get(..3) else { return false };
    let mut tail = stem.get(3..).unwrap_or_default().chars();
    (head.eq_ignore_ascii_case("COM") || head.eq_ignore_ascii_case("LPT"))
        && matches!((tail.next(), tail.next()), (Some('0'..='9' | '¹' | '²' | '³'), None))
}

/// Fold `.` and `..`. A rooted path may not climb above its root; a relative one keeps leading `..`.
fn normalize(root: &Root, comps: Vec<String>) -> Result<Vec<String>, Refused> {
    let mut out: Vec<String> = Vec::with_capacity(comps.len());
    for c in comps {
        match c.as_str() {
            "." => {}
            ".." => match out.last() {
                Some(last) if last != ".." => {
                    out.pop();
                }
                _ if root.is_absolute() => return Err(Refused::Invalid),
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    Ok(out)
}

fn render(root: &Root, comps: &[String]) -> PathBuf {
    let (prefix, sep) = match root {
        Root::Relative if comps.is_empty() => (".".to_string(), MAIN_SEPARATOR),
        Root::Relative => (String::new(), MAIN_SEPARATOR),
        Root::Rooted => (MAIN_SEPARATOR.to_string(), MAIN_SEPARATOR),
        Root::DriveRelative(l) => (format!("{l}:"), '\\'),
        Root::Drive(l) => (format!("{l}:\\"), '\\'),
        Root::VerbatimDrive(l) => (format!(r"\\?\{l}:\"), '\\'),
        Root::Share { host, share, verbatim: false } => (format!(r"\\{host}\{share}\"), '\\'),
        Root::Share { host, share, verbatim: true } => (format!(r"\\?\UNC\{host}\{share}\"), '\\'),
    };
    PathBuf::from(prefix + &comps.join(&sep.to_string()))
}

/// A relative link next to a document whose path is not UTF-8 (possible on Unix only).
fn join_native(doc: &Path, comps: &[String]) -> Result<PathBuf, Refused> {
    let mut p = doc.parent().map(Path::to_path_buf).unwrap_or_default();
    for c in comps {
        match c.as_str() {
            "." => {}
            ".." => {
                if !p.pop() {
                    return Err(Refused::Invalid);
                }
            }
            _ => p.push(c),
        }
    }
    Ok(p)
}
