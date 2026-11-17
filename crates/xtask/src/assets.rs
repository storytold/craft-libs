//! The asset attribution check: every asset file in the repository has a row in the app's
//! attribution document.
//!
//! The app supplies a [`Policy`]: the attribution document's path, how its rows are read
//! ([`RowMode`]), the extensions it adds to [`ASSET_EXT`], the folders whose every file is an asset,
//! the extensions that count only inside named folders, and the extensions and file-name fragments
//! it does not allow. [`run`] lists the repository's files through Git ([`list_files`]), reads the
//! document and calls [`check`]. The app keeps its `cargo xtask assets` command, the printing and
//! the gate step.
//!
//! ```
//! use craft_xtask::assets::{self, Policy, Violation};
//!
//! const POLICY: Policy<'static> =
//!     Policy { extra_ext: &["appdoc"], all_files_under: &["assets", "docs/images"], ..Policy::new("ASSETS.md") };
//!
//! let files = ["assets/icons/a.svg", "docs/images/b.png", "src/lib.rs", "tests/c.appdoc"].map(String::from);
//! let md = "| Path | Author | License |\n|---|---|---|\n| `assets/icons/a.svg` | me | MIT |\n";
//! let report = assets::check(&files, md, &POLICY, |_| true)?;
//! assert_eq!(report.assets, 3);
//! let missing: Vec<String> = report.violations.iter().map(Violation::to_string).collect();
//! assert_eq!(missing, ["`docs/images/b.png` has no row in the attribution document", "`tests/c.appdoc` has no row in the attribution document"]);
//! # Ok::<(), assets::Error>(())
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{Command, Stdio};

/// The extensions that make a file an asset wherever it is, lower case: images, icons, raw camera
/// files, color profiles, fonts, swatch and brush files, documents, sound and video.
pub const ASSET_EXT: &[&str] = &[
    "abr",
    "aco",
    "ai",
    "arw",
    "ase",
    "avif",
    "bmp",
    "cr2",
    "cr3",
    "cube",
    "deckcraft",
    "designcraft",
    "dng",
    "docx",
    "drawcraft",
    "dwg",
    "dxf",
    "eps",
    "flac",
    "gif",
    "heic",
    "icc",
    "icm",
    "icns",
    "ico",
    "idml",
    "indd",
    "jpeg",
    "jpg",
    "jxl",
    "m4a",
    "mov",
    "mp3",
    "mp4",
    "nef",
    "odt",
    "ogg",
    "orf",
    "otc",
    "otf",
    "pdf",
    "pef",
    "png",
    "potx",
    "ppsx",
    "pptx",
    "psd",
    "raf",
    "raw",
    "rtf",
    "rw2",
    "rwl",
    "srw",
    "svg",
    "thmx",
    "tif",
    "tiff",
    "ttc",
    "ttf",
    "vectorcraft",
    "wav",
    "webm",
    "webp",
    "woff",
    "woff2",
    "xls",
    "xlsm",
    "xlsx",
    "xmp",
];

/// The default limit on the attribution document's size: 8 MiB.
pub const DEFAULT_DOCUMENT_LIMIT: u64 = 8 * 1024 * 1024;

/// The default limit on the bytes [`list_files`] reads from `git ls-files`: 32 MiB.
pub const DEFAULT_GIT_OUTPUT_LIMIT: usize = 32 * 1024 * 1024;

/// How the attribution document's rows name the files they cover.
///
/// Either way, only the backticked spans in the first cell of a Markdown table row count: a line
/// that starts with `|`, or a line of a table whose second line is a delimiter row (`---|---`), up
/// to the next blank line. Lines in a fenced code block, and lines indented 4 columns or more, are
/// not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowMode {
    /// A span covers the one file whose repository path it spells exactly.
    Exact,
    /// A span is a pattern: `*` matches any run of characters except `/`, a span ending in `/`
    /// covers every file below that folder, and every other character matches itself.
    Patterns,
}

/// Extensions that make a file an asset only inside a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderExtensions<'a> {
    /// The folder, a repository path such as `examples` or `docs/samples`.
    pub folder: &'a str,
    /// When `true`, a folder with this path at any depth counts, for example
    /// `crates/engine/examples` for `examples`. When `false`, only the folder at the repository
    /// root counts.
    pub nested: bool,
    /// The extensions, without the dot. They compare without regard to ASCII case.
    pub extensions: &'a [&'a str],
}

/// What the app counts as an asset and how its attribution document is read.
///
/// Start from [`Policy::new`] and set the fields the app needs. Folder paths use `/` and are
/// relative to the repository root; a trailing `/` is allowed. Extensions are given without the
/// dot and compare without regard to ASCII case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy<'a> {
    /// The attribution document, relative to the repository root, such as `ASSETS.md` or
    /// `assets/ATTRIBUTION.md`.
    pub attribution_file: &'a str,
    /// How the document's rows name files. [`RowMode::Exact`] by default.
    pub row_mode: RowMode,
    /// Extensions that make a file an asset anywhere, on top of [`ASSET_EXT`].
    pub extra_ext: &'a [&'a str],
    /// Folders whose every file is an asset, whatever its extension.
    pub all_files_under: &'a [&'a str],
    /// Extensions that make a file an asset only inside a folder.
    pub ext_in_folders: &'a [FolderExtensions<'a>],
    /// Folders whose files are not checked, except against [`Policy::forbidden_ext`].
    /// [`Report::skipped_by_folder`] counts them.
    pub skip_folders: &'a [&'a str],
    /// Extensions no file may have, asset or not. A file with one is reported as
    /// [`Violation::ForbiddenExtension`] and not checked further.
    pub forbidden_ext: &'a [&'a str],
    /// Text an asset's file name may not contain, compared without regard to ASCII case.
    pub forbidden_name_fragments: &'a [&'a str],
    /// The zero-based table column that names license files, when the app checks them. Every
    /// backticked span in that cell that contains `/` must name an existing file. Column 0 is the
    /// path column and is not allowed here.
    pub license_column: Option<usize>,
    /// The largest attribution document [`run`] reads, in bytes.
    pub max_document_bytes: u64,
    /// The most bytes [`run`] reads from `git ls-files`.
    pub max_git_output_bytes: usize,
}

impl<'a> Policy<'a> {
    /// A policy with the given attribution document, [`RowMode::Exact`], no extra extensions or
    /// folders, nothing forbidden, no license column and the default limits.
    pub const fn new(attribution_file: &'a str) -> Self {
        Policy {
            attribution_file,
            row_mode: RowMode::Exact,
            extra_ext: &[],
            all_files_under: &[],
            ext_in_folders: &[],
            skip_folders: &[],
            forbidden_ext: &[],
            forbidden_name_fragments: &[],
            license_column: None,
            max_document_bytes: DEFAULT_DOCUMENT_LIMIT,
            max_git_output_bytes: DEFAULT_GIT_OUTPUT_LIMIT,
        }
    }
}

/// One finding of the check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// An asset no row covers.
    Unattributed {
        /// The asset's repository path.
        path: String,
        /// A row entry that spells the path with different letter case, if there is one.
        near: Option<String>,
    },
    /// A file with an extension the policy does not allow.
    ForbiddenExtension {
        /// The file's repository path.
        path: String,
        /// The extension, lower case.
        ext: String,
    },
    /// An asset whose file name contains a fragment the policy does not allow.
    ForbiddenName {
        /// The asset's repository path.
        path: String,
        /// The fragment, as the policy gives it.
        fragment: String,
    },
    /// A row names a license file that does not exist.
    MissingLicense {
        /// The row's line number in the document, from 1.
        line: usize,
        /// The license file's repository path.
        file: String,
    },
    /// A row entry that cannot name a repository path.
    BadEntry {
        /// The row's line number in the document, from 1.
        line: usize,
        /// The backticked text.
        entry: String,
        /// Why it cannot name a path.
        reason: &'static str,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Unattributed { path, near: None } => write!(f, "`{path}` has no row in the attribution document"),
            Violation::Unattributed { path, near: Some(near) } => {
                write!(f, "`{path}` has no row in the attribution document; the row `{near}` differs from it only in letter case")
            }
            Violation::ForbiddenExtension { path, ext } => write!(f, "`{path}`: `.{ext}` files are not allowed"),
            Violation::ForbiddenName { path, fragment } => write!(f, "`{path}`: the file name contains `{fragment}`, which is not allowed"),
            Violation::MissingLicense { line, file } => write!(f, "line {line}: the license file `{file}` does not exist"),
            Violation::BadEntry { line, entry, reason } => write!(f, "line {line}: `{entry}` {reason}"),
        }
    }
}

/// The result of a check.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The number of asset files checked.
    pub assets: usize,
    /// The number of files in [`Policy::skip_folders`].
    pub skipped_by_folder: usize,
    /// The number of Git entries [`run`] passed over because they are not regular files (symbolic
    /// links, submodule folders and other special files) or are reached through a linked folder.
    /// [`check`] leaves it at 0.
    pub skipped_entries: usize,
    /// The findings, in file order, then in document order.
    pub violations: Vec<Violation>,
}

impl Report {
    /// `true` when the check found nothing.
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Why the check could not run.
#[derive(Debug)]
pub enum Error {
    /// The policy cannot be used: an invalid path, an empty extension, a zero limit, license column 0.
    Policy(String),
    /// `git ls-files` could not be started, failed, or wrote more than the byte limit.
    Git(String),
    /// `git ls-files` returned a name that is not UTF-8 or that a row cannot name (see [`parse_ls_files`]).
    GitName(String),
    /// The attribution document could not be read, is larger than the limit, or is not UTF-8.
    Document(String),
    /// A listed file's metadata could not be read.
    Metadata(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Policy(m) => write!(f, "asset policy: {m}"),
            Error::Git(m) | Error::GitName(m) => write!(f, "git ls-files: {m}"),
            Error::Document(m) | Error::Metadata(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

/// The files [`list_files`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    /// The regular files' repository paths, sorted, without duplicates.
    pub files: Vec<String>,
    /// The number of entries passed over because they are not regular files or are reached through
    /// a linked folder.
    pub skipped_entries: usize,
}

/// Why a path cannot be a repository-relative path with `/` separators, or `None` when it can.
fn path_problem(path: &str) -> Option<&'static str> {
    if path.contains('\\') {
        return Some("uses `\\`; repository paths use `/`");
    }
    // `C:x` is relative to drive C's current folder on Windows, not to the repository.
    let drive = matches!(path.as_bytes(), [letter, b':', ..] if letter.is_ascii_alphabetic());
    if path.starts_with('/') || drive || path.split('/').next().is_some_and(|first| first.ends_with(':')) {
        return Some("is an absolute path or starts with a drive");
    }
    if path.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return Some("has an empty, `.` or `..` component");
    }
    None
}

/// The folder without one trailing `/`.
fn folder_key(folder: &str) -> &str {
    folder.strip_suffix('/').unwrap_or(folder)
}

fn check_folder(field: &str, folder: &str) -> Result<(), Error> {
    match path_problem(folder_key(folder)) {
        Some(reason) => Err(Error::Policy(format!("{field} entry `{folder}` {reason}"))),
        None => Ok(()),
    }
}

fn check_ext(field: &str, ext: &str) -> Result<(), Error> {
    if ext.is_empty() || ext.contains(['.', '/', '\\']) {
        return Err(Error::Policy(format!("{field} entry `{ext}` must be an extension without the dot")));
    }
    Ok(())
}

/// Checks the policy's paths, extensions and limits.
pub fn validate(policy: &Policy<'_>) -> Result<(), Error> {
    if policy.attribution_file.ends_with('/') {
        return Err(Error::Policy(format!("attribution_file `{}` names a folder", policy.attribution_file)));
    }
    if let Some(reason) = path_problem(policy.attribution_file) {
        return Err(Error::Policy(format!("attribution_file `{}` {reason}", policy.attribution_file)));
    }
    for folder in policy.all_files_under {
        check_folder("all_files_under", folder)?;
    }
    for folder in policy.skip_folders {
        check_folder("skip_folders", folder)?;
    }
    for rule in policy.ext_in_folders {
        check_folder("ext_in_folders", rule.folder)?;
        rule.extensions.iter().try_for_each(|e| check_ext("ext_in_folders", e))?;
    }
    policy.extra_ext.iter().try_for_each(|e| check_ext("extra_ext", e))?;
    policy.forbidden_ext.iter().try_for_each(|e| check_ext("forbidden_ext", e))?;
    if policy.forbidden_name_fragments.iter().any(|f| f.is_empty()) {
        return Err(Error::Policy("forbidden_name_fragments has an empty entry".into()));
    }
    if policy.license_column == Some(0) {
        return Err(Error::Policy("license_column 0 is the path column".into()));
    }
    if policy.max_document_bytes == 0 || policy.max_git_output_bytes == 0 {
        return Err(Error::Policy("the size limits must be above 0".into()));
    }
    Ok(())
}

/// `true` when `path` is `folder` itself or below it, comparing whole components.
fn under(path: &str, folder: &str) -> bool {
    path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// `true` when `path` is below a folder `folder` at any depth.
fn under_anywhere(path: &str, folder: &str) -> bool {
    under(path, folder) || path.match_indices('/').any(|(i, _)| under(&path[i + 1..], folder))
}

/// The file name's extension, lower case, as [`Path::extension`] reads it.
fn extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    Path::new(name).extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase)
}

fn has_ext(list: &[&str], ext: &str) -> bool {
    list.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// `true` when the policy counts `path` as an asset. Skipped folders and forbidden extensions are
/// not considered here.
pub fn is_asset(policy: &Policy<'_>, path: &str) -> bool {
    if policy.all_files_under.iter().any(|d| under(path, folder_key(d))) {
        return true;
    }
    let Some(ext) = extension(path) else { return false };
    if has_ext(ASSET_EXT, &ext) || has_ext(policy.extra_ext, &ext) {
        return true;
    }
    policy.ext_in_folders.iter().any(|rule| {
        let folder = folder_key(rule.folder);
        has_ext(rule.extensions, &ext) && if rule.nested { under_anywhere(path, folder) } else { under(path, folder) }
    })
}

/// `*` matches any run of characters except `/`; every other character matches itself. The table
/// over the pattern costs pattern length times name length, however many `*` the pattern holds.
fn glob(pat: &[char], name: &[char]) -> bool {
    // `row[k]`: the pattern so far matches the first k characters of the name.
    let mut row = vec![false; name.len() + 1];
    let mut next = vec![false; name.len() + 1];
    row[0] = true;
    for &pc in pat {
        next[0] = pc == '*' && row[0];
        for k in 1..=name.len() {
            let c = name[k - 1];
            next[k] = if pc == '*' { row[k] || (next[k - 1] && c != '/') } else { row[k - 1] && c == pc };
        }
        std::mem::swap(&mut row, &mut next);
    }
    row[name.len()]
}

/// The rows of the attribution document.
#[derive(Default)]
struct Rows {
    exact: HashSet<String>,
    /// Lower-case spelling to entry, for the near-miss hint.
    lower: HashMap<String, String>,
    dirs: Vec<String>,
    dir_globs: Vec<Vec<char>>,
    globs: Vec<Vec<char>>,
    licenses: Vec<(usize, String)>,
    bad: Vec<Violation>,
}

impl Rows {
    fn add(&mut self, mode: RowMode, line: usize, entry: &str) {
        let (body, dir) = match (mode, entry.strip_suffix('/')) {
            (RowMode::Patterns, Some(body)) => (body, true),
            _ => (entry, false),
        };
        if let Some(reason) = path_problem(body) {
            self.bad.push(Violation::BadEntry { line, entry: entry.to_string(), reason });
            return;
        }
        let wild = mode == RowMode::Patterns && body.contains('*');
        match (dir, wild) {
            (true, false) => self.dirs.push(body.to_string()),
            (true, true) => self.dir_globs.push(body.chars().collect()),
            (false, true) => self.globs.push(body.chars().collect()),
            (false, false) => {
                self.lower.entry(body.to_ascii_lowercase()).or_insert_with(|| body.to_string());
                self.exact.insert(body.to_string());
            }
        }
    }

    fn add_license(&mut self, line: usize, entry: &str) {
        match path_problem(entry) {
            Some(reason) => self.bad.push(Violation::BadEntry { line, entry: entry.to_string(), reason }),
            None => self.licenses.push((line, entry.to_string())),
        }
    }

    fn covers(&self, path: &str) -> bool {
        if self.exact.contains(path) || self.dirs.iter().any(|d| under(path, d)) {
            return true;
        }
        if self.globs.is_empty() && self.dir_globs.is_empty() {
            return false;
        }
        let name: Vec<char> = path.chars().collect();
        if self.globs.iter().any(|g| glob(g, &name)) {
            return true;
        }
        // A folder pattern covers the path when it matches one of the path's folders.
        name.iter().enumerate().filter(|&(_, &c)| c == '/').any(|(i, _)| self.dir_globs.iter().any(|g| glob(g, &name[..i])))
    }
}

/// A code span: the byte range of the whole span, backticks included, and of its text.
struct Span {
    outer: std::ops::Range<usize>,
    text: std::ops::Range<usize>,
}

/// The code spans of `s`, in order: a run of N backticks opens a span that the next run of exactly
/// N backticks closes, and a run with no closing run is plain text. A backslash does not escape a
/// backtick here. One space is stripped from each end of the text when both ends have one and the
/// text is not all spaces. The cost is linear in the length of `s` apart from a binary search per
/// run.
fn code_spans(s: &str) -> Vec<Span> {
    let b = s.as_bytes();
    // (start, length) of each run of backticks.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let n = b[i..].iter().take_while(|&&c| c == b'`').count();
        if n > 0 {
            runs.push((i, n));
        }
        i += n.max(1);
    }
    // The indexes into `runs` of the runs of each length.
    let mut by_len: HashMap<usize, Vec<usize>> = HashMap::new();
    for (r, &(_, n)) in runs.iter().enumerate() {
        by_len.entry(n).or_default().push(r);
    }
    let mut spans = Vec::new();
    let mut r = 0;
    while r < runs.len() {
        let (open, n) = runs[r];
        let close = by_len.get(&n).and_then(|list| list.get(list.partition_point(|&x| x <= r)).copied());
        let Some(c) = close else {
            r += 1;
            continue;
        };
        let (mut start, mut end) = (open + n, runs[c].0);
        let text = &s[start..end];
        if text.len() >= 2 && text.starts_with(' ') && text.ends_with(' ') && !text.bytes().all(|x| x == b' ') {
            start += 1;
            end -= 1;
        }
        spans.push(Span { outer: open..runs[c].0 + n, text: start..end });
        r = c + 1;
    }
    spans
}

/// The cells of a table row, split on `|` outside code spans and not escaped with `\`. The empty
/// text before a leading `|` and after a trailing one is dropped.
fn cells(line: &str) -> Vec<&str> {
    let spans = code_spans(line);
    let mut out = Vec::new();
    let (mut start, mut escaped, mut next_span) = (0, false, 0);
    for (i, c) in line.char_indices() {
        while spans.get(next_span).is_some_and(|s| s.outer.end <= i) {
            next_span += 1;
        }
        if spans.get(next_span).is_some_and(|s| s.outer.contains(&i)) {
            escaped = false;
            continue;
        }
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '|' => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    // A leading `|` leaves an empty first piece; a trailing `|` leaves an empty last one.
    if out.len() > 1 && line.trim_start().starts_with('|') {
        out.remove(0);
    }
    if out.len() > 1 && out.last().is_some_and(|c| c.trim().is_empty()) {
        out.pop();
    }
    out
}

/// The texts of the code spans of a cell.
fn backticked(cell: &str) -> impl Iterator<Item = &str> {
    code_spans(cell).into_iter().map(move |s| &cell[s.text])
}

/// The indentation of a line in columns, a tab counting as 4.
fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// A code fence marker at the start of `line` (already without its indentation): the fence
/// character, the length of the run of it, and the text after the run.
fn fence_marker(line: &str) -> Option<(char, usize, &str)> {
    let c = line.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = line.chars().take_while(|x| *x == c).count();
    (len >= 3).then(|| (c, len, &line[len..]))
}

/// `true` for a table's delimiter row, such as `|---|:--:|` or `--- | ---`.
fn is_delimiter_row(line: &str) -> bool {
    let t = line.trim();
    if indent(line) >= 4 || !t.contains('|') {
        return false;
    }
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').all(|cell| {
        let c = cell.trim();
        let c = c.strip_prefix(':').unwrap_or(c);
        let c = c.strip_suffix(':').unwrap_or(c);
        !c.is_empty() && c.bytes().all(|b| b == b'-')
    })
}

/// The table rows of the document, trimmed, with their line numbers from 1. A table row is a line
/// that starts with `|`, or any line of a table whose second line is a delimiter row, up to the
/// next blank line. Lines in fenced code blocks and lines indented 4 columns or more are left out.
fn table_rows(md: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = Vec::new();
    // The open fence's character and length.
    let mut fence: Option<(char, usize)> = None;
    let mut table = false;
    for (i, raw) in lines.iter().enumerate() {
        // A line indented 4 columns or more is an indented code block or inside one.
        let code = indent(raw) >= 4;
        let line = raw.trim();
        let marker = if code { None } else { fence_marker(line) };
        match (fence, marker) {
            (None, Some((c, len, _))) => fence = Some((c, len)),
            (Some((open, open_len)), Some((c, len, rest))) if c == open && len >= open_len && rest.trim().is_empty() => fence = None,
            _ => {}
        }
        if code || fence.is_some() || marker.is_some() || line.is_empty() {
            table = false;
            continue;
        }
        if !table && line.contains('|') && lines.get(i + 1).is_some_and(|next| is_delimiter_row(next)) {
            table = true;
        }
        if table || line.starts_with('|') {
            out.push((i + 1, line));
        }
    }
    out
}

fn parse_rows(md: &str, policy: &Policy<'_>) -> Rows {
    let mut rows = Rows::default();
    for (line_no, line) in table_rows(md) {
        let cells = cells(line);
        for entry in cells.first().into_iter().flat_map(|c| backticked(c)) {
            rows.add(policy.row_mode, line_no, entry);
        }
        if let Some(cell) = policy.license_column.and_then(|n| cells.get(n)) {
            for entry in backticked(cell).filter(|e| e.contains('/')) {
                rows.add_license(line_no, entry);
            }
        }
    }
    rows
}

/// Checks `files` (repository paths with `/`) against the attribution document `md`.
/// `license_exists` answers whether a license file a row names exists; it is called only when
/// [`Policy::license_column`] is set.
///
/// Returns an error only for a policy [`validate`] rejects.
pub fn check(files: &[String], md: &str, policy: &Policy<'_>, license_exists: impl Fn(&str) -> bool) -> Result<Report, Error> {
    validate(policy)?;
    let rows = parse_rows(md, policy);
    let mut report = Report::default();
    for path in files {
        if let Some(ext) = extension(path).filter(|e| has_ext(policy.forbidden_ext, e)) {
            report.violations.push(Violation::ForbiddenExtension { path: path.clone(), ext });
            continue;
        }
        if policy.skip_folders.iter().any(|d| under(path, folder_key(d))) {
            report.skipped_by_folder += 1;
            continue;
        }
        if !is_asset(policy, path) {
            continue;
        }
        report.assets += 1;
        let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
        if let Some(fragment) = policy.forbidden_name_fragments.iter().find(|f| name.contains(&f.to_ascii_lowercase())) {
            report.violations.push(Violation::ForbiddenName { path: path.clone(), fragment: (*fragment).to_string() });
        }
        if !rows.covers(path) {
            let near = rows.lower.get(&path.to_ascii_lowercase()).cloned();
            report.violations.push(Violation::Unattributed { path: path.clone(), near });
        }
    }
    report.violations.extend(rows.bad);
    for (line, file) in rows.licenses {
        if !license_exists(&file) {
            report.violations.push(Violation::MissingLicense { line, file });
        }
    }
    Ok(report)
}

/// Splits `git ls-files -z` output into names. Each name must be UTF-8 and a path a row can name:
/// relative, with `/` separators, no `\`, no drive prefix and no empty, `.` or `..` component. The
/// names are returned sorted and without duplicates.
pub fn parse_ls_files(out: &[u8]) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    for raw in out.split(|&b| b == 0).filter(|n| !n.is_empty()) {
        let name = std::str::from_utf8(raw).map_err(|_| Error::GitName(format!("the name `{}` is not UTF-8", String::from_utf8_lossy(raw))))?;
        if let Some(reason) = path_problem(name) {
            return Err(Error::GitName(format!("the name `{name}` {reason}")));
        }
        names.push(name.to_string());
    }
    // An unmerged file is listed once for each stage.
    names.sort_unstable();
    names.dedup();
    Ok(names)
}

/// Lists the files Git tracks in `root`, and the untracked files it does not ignore, through
/// `git ls-files -z --cached --others --exclude-standard`.
///
/// Reads at most `max_bytes` of its output (the child is ended when it writes more). Names that no
/// longer exist on disk are left out, as are symbolic links, submodule folders, other entries that
/// are not regular files, and files reached through a linked folder; links are not followed.
/// Stderr is inherited.
pub fn list_files(root: &Path, max_bytes: usize) -> Result<Listing, Error> {
    let mut cmd = Command::new("git");
    cmd.current_dir(root).args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| Error::Git(format!("failed to spawn: {e}")))?;
    let mut buf = Vec::new();
    let read = match child.stdout.take() {
        Some(stdout) => stdout.take(u64::try_from(max_bytes).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut buf).map(|_| ()),
        None => Err(std::io::Error::other("no stdout pipe")),
    };
    let failure = match read {
        Err(e) => Some(format!("reading its output failed: {e}")),
        Ok(()) if buf.len() > max_bytes => Some(format!("wrote more than {max_bytes} bytes")),
        Ok(()) => None,
    };
    if let Some(message) = failure {
        // The child may still be writing; end it so that the wait returns.
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::Git(message));
    }
    let status = child.wait().map_err(|e| Error::Git(format!("waiting for it failed: {e}")))?;
    if !status.success() {
        return Err(Error::Git(format!("exited with {status}")));
    }
    let mut listing = Listing::default();
    let mut folders = HashSet::new();
    for name in parse_ls_files(&buf)? {
        match entry(root, &name, &mut folders) {
            Ok(Entry::File) => listing.files.push(name),
            Ok(Entry::Other) => listing.skipped_entries += 1,
            Ok(Entry::Missing) => {}
            Err(e) => return Err(Error::Metadata(format!("{name}: {e}"))),
        }
    }
    Ok(listing)
}

/// Reads the attribution document, failing before the read when it is larger than `max_bytes`.
pub fn read_document(root: &Path, file: &str, max_bytes: u64) -> Result<String, Error> {
    let fail = |e: std::io::Error| Error::Document(format!("{file}: {e}"));
    let handle = std::fs::File::open(root.join(file)).map_err(fail)?;
    if handle.metadata().map_err(fail)?.len() > max_bytes {
        return Err(Error::Document(format!("{file}: larger than {max_bytes} bytes")));
    }
    let mut bytes = Vec::new();
    handle.take(max_bytes.saturating_add(1)).read_to_end(&mut bytes).map_err(fail)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(Error::Document(format!("{file}: larger than {max_bytes} bytes")));
    }
    String::from_utf8(bytes).map_err(|_| Error::Document(format!("{file}: not UTF-8")))
}

/// What a repository path is on disk, read without following links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// The path or a folder on the way to it does not exist.
    Missing,
    /// A regular file, reached through real folders only.
    File,
    /// A symbolic link, a folder or another kind of entry, or a path through a link.
    Other,
}

/// Reads what `path` (relative, with `/`) is below `root`, checking every folder on the way with
/// `symlink_metadata`. `folders` holds the folders already found to be real ones.
fn entry(root: &Path, path: &str, folders: &mut HashSet<String>) -> std::io::Result<Entry> {
    let mut parts = path.match_indices('/').map(|(i, _)| i).chain([path.len()]).peekable();
    while let Some(end) = parts.next() {
        let last = parts.peek().is_none();
        let prefix = &path[..end];
        if !last && folders.contains(prefix) {
            continue;
        }
        let kind = match std::fs::symlink_metadata(root.join(prefix)) {
            Ok(meta) => meta.file_type(),
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => return Ok(Entry::Missing),
            Err(e) => return Err(e),
        };
        match (last, kind.is_dir(), kind.is_file()) {
            (false, true, _) => {
                folders.insert(prefix.to_string());
            }
            (true, _, true) => return Ok(Entry::File),
            _ => return Ok(Entry::Other),
        }
    }
    Ok(Entry::Missing)
}

/// Lists the repository's files ([`list_files`]), reads the attribution document
/// ([`read_document`]) and checks them ([`check`]). A license file a row names must be a regular
/// file in the repository; neither it nor a folder on the way to it may be a symbolic link.
pub fn run(root: &Path, policy: &Policy<'_>) -> Result<Report, Error> {
    validate(policy)?;
    let listing = list_files(root, policy.max_git_output_bytes)?;
    let md = read_document(root, policy.attribution_file, policy.max_document_bytes)?;
    let folders = std::cell::RefCell::new(HashSet::new());
    let mut report = check(&listing.files, &md, policy, |p| matches!(entry(root, p, &mut folders.borrow_mut()), Ok(Entry::File)))?;
    report.skipped_entries = listing.skipped_entries;
    Ok(report)
}

#[cfg(test)]
mod tests;
