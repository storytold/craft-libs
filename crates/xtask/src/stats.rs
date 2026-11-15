//! `cargo xtask stats`: files, lines and tests of Rust per workspace crate.
//!
//! [`run`] reads the workspace packages from the `cargo metadata --format-version 1 --no-deps` JSON the
//! caller passes in, walks each package's folder and writes a table to the caller's writer:
//!
//! ```text
//! crate                       files    lines #[test]  proptest!
//! --------------------------------------------------------------
//! app-core                       12     3410      41          2
//! --------------------------------------------------------------
//! TOTAL                          12     3410      41          2
//! ```
//!
//! What it counts:
//!
//! - Every regular file whose name ends in `.rs`, in the package's folder and the folders below it.
//!   Folders named `target` or `fuzz`, and folders whose names start with `.`, are skipped. The
//!   folder of another workspace member is counted in that member's row only.
//!   Symbolic links (and Windows junctions) are not followed, to folders or to files.
//! - Lines as [`str::lines`] counts them, blank and comment lines included.
//! - `#[test]` is an estimate read from the text: each `#[test]` on a line that does not start with
//!   `//`. `#[tokio::test]` and the tests inside `proptest!` are not counted; each line that starts
//!   with `proptest!` counts as one block in its own column.
//! - With [`Options::exact`], the harness column runs `cargo test -q -p <crate> -- --list` for each
//!   crate and counts the output lines that end in `: test`. A crate whose command fails shows
//!   `error`, the harness total leaves it out, and a line under the table says how many failed.
//!
//! Every input has a size limit ([`Limits`]); an input over a limit, a folder or file that cannot be
//! read, and a source that is not UTF-8 return an [`Error`] naming the path, and nothing is written.

use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Size limits for one [`run`]. An input over a limit is an [`Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of `cargo metadata` JSON, checked before it is parsed. Default 64 MiB.
    pub metadata_bytes: usize,
    /// Bytes of one `.rs` file. Default 16 MiB.
    pub source_bytes: u64,
    /// Bytes of all `.rs` files read in one run. Default 1 GiB.
    pub total_source_bytes: u64,
    /// Folder entries visited in one run, all crates together. Default 1,000,000.
    pub entries: usize,
    /// Levels of folders below a crate's folder. Default 256.
    pub depth: usize,
    /// Bytes of standard output from one crate's `cargo test -- --list`, in exact mode. Default 64 MiB.
    pub harness_output_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            metadata_bytes: 64 << 20,
            source_bytes: 16 << 20,
            total_source_bytes: 1 << 30,
            entries: 1_000_000,
            depth: 256,
            harness_output_bytes: 64 << 20,
        }
    }
}

/// How [`run`] counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Ask each crate's test harness for its test count (`--exact`).
    pub exact: bool,
    /// The size limits.
    pub limits: Limits,
}

/// Counts for one crate, or for all of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// `.rs` files.
    pub files: usize,
    /// Lines in those files.
    pub lines: usize,
    /// `#[test]` attributes found in the text.
    pub tests: usize,
    /// Lines that start a `proptest!` block.
    pub proptests: usize,
}

impl Counts {
    fn add(&mut self, other: &Counts) {
        self.files = self.files.saturating_add(other.files);
        self.lines = self.lines.saturating_add(other.lines);
        self.tests = self.tests.saturating_add(other.tests);
        self.proptests = self.proptests.saturating_add(other.proptests);
    }

    fn add_source(&mut self, src: &str) {
        let (tests, proptests) = count_tests(src);
        self.add(&Counts { files: 1, lines: src.lines().count(), tests, proptests });
    }
}

/// The result of one crate's `cargo test -- --list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Harness {
    /// The command succeeded and listed this many tests.
    Listed(usize),
    /// The command could not start or did not succeed; the text says why.
    Failed(String),
}

/// One crate's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The package name.
    pub name: String,
    /// The package's `Cargo.toml`, as `cargo metadata` gives it.
    pub manifest_path: PathBuf,
    /// What the walk of the package's folder found.
    pub counts: Counts,
    /// The harness count, in exact mode only.
    pub harness: Option<Harness>,
}

/// Every crate's counts, sorted by package name and then by manifest path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// One row per workspace package.
    pub rows: Vec<Row>,
    /// The sum of the rows' counts.
    pub total: Counts,
    /// Whether the harness counts were asked for.
    pub exact: bool,
}

impl Report {
    /// The sum of the harness counts that succeeded, or `None` outside exact mode.
    pub fn harness_total(&self) -> Option<usize> {
        self.exact.then(|| {
            self.rows
                .iter()
                .filter_map(|r| match r.harness {
                    Some(Harness::Listed(n)) => Some(n),
                    _ => None,
                })
                .fold(0, usize::saturating_add)
        })
    }

    /// The rows whose harness command failed.
    pub fn harness_failures(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| matches!(r.harness, Some(Harness::Failed(_))))
    }
}

/// Why [`run`] stopped.
#[derive(Debug)]
pub enum Error {
    /// The `cargo metadata` JSON is too large, is not JSON, or lacks a field.
    Metadata(String),
    /// A folder or file could not be read.
    Io {
        /// The folder or file.
        path: PathBuf,
        /// The error reading it.
        source: io::Error,
    },
    /// A `.rs` file is not UTF-8.
    NotUtf8(PathBuf),
    /// A `.rs` file is larger than [`Limits::source_bytes`].
    SourceTooLarge {
        /// The file.
        path: PathBuf,
        /// The limit in bytes.
        limit: u64,
    },
    /// Reading a file would take the run past [`Limits::total_source_bytes`].
    TotalTooLarge {
        /// The file being read.
        path: PathBuf,
        /// The limit in bytes.
        limit: u64,
    },
    /// The run visited [`Limits::entries`] folder entries.
    TooManyEntries {
        /// The folder being read.
        path: PathBuf,
        /// The limit.
        limit: usize,
    },
    /// A folder is more than [`Limits::depth`] levels below its crate's folder.
    TooDeep {
        /// The folder.
        path: PathBuf,
        /// The limit.
        limit: usize,
    },
    /// A crate's `cargo test -- --list` printed more than [`Limits::harness_output_bytes`].
    HarnessOutputTooLarge {
        /// The package name.
        package: String,
        /// The limit in bytes.
        limit: u64,
    },
    /// The table could not be written.
    Write(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Metadata(why) => write!(f, "cargo metadata: {why}"),
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::NotUtf8(path) => write!(f, "{}: not UTF-8", path.display()),
            Error::SourceTooLarge { path, limit } => write!(f, "{}: larger than the limit of {limit} bytes for one source", path.display()),
            Error::TotalTooLarge { path, limit } => {
                write!(f, "{}: reading it passes the limit of {limit} bytes for all sources", path.display())
            }
            Error::TooManyEntries { path, limit } => write!(f, "{}: the walk reached its limit of {limit} folder entries", path.display()),
            Error::TooDeep { path, limit } => write!(f, "{}: more than {limit} folders deep", path.display()),
            Error::HarnessOutputTooLarge { package, limit } => {
                write!(f, "cargo test -p {package} -- --list: printed more than the limit of {limit} bytes")
            }
            Error::Write(e) => write!(f, "writing the table: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } | Error::Write(source) => Some(source),
            _ => None,
        }
    }
}

/// Counts every workspace crate in `metadata_json` and writes the table to `out`.
///
/// `cargo` builds the app's `cargo` command (program, folder and environment); in exact mode the
/// arguments `test -q -p <crate> -- --list` are added to it for each crate. Outside exact mode it is
/// not called. Nothing is written when an [`Error`] is returned before the table.
pub fn run(metadata_json: &[u8], options: &Options, cargo: impl FnMut() -> Command, out: &mut impl Write) -> Result<Report, Error> {
    let report = collect(metadata_json, options, cargo)?;
    write_table(&report, out).map_err(Error::Write)?;
    Ok(report)
}

/// Counts every workspace crate in `metadata_json`, as [`run`] does, without writing anything.
pub fn collect(metadata_json: &[u8], options: &Options, mut cargo: impl FnMut() -> Command) -> Result<Report, Error> {
    let limits = &options.limits;
    let packages = packages(metadata_json, limits.metadata_bytes)?;
    let mut dirs = Vec::with_capacity(packages.len());
    for package in &packages {
        let dir = package
            .manifest_path
            .parent()
            .ok_or_else(|| Error::Metadata(format!("package {}: manifest path {} has no folder", package.name, package.manifest_path.display())))?;
        dirs.push(dir.to_path_buf());
    }
    let members = dirs.iter().cloned().collect();
    let mut walker = Walker { limits, members, entries: 0, bytes: 0 };
    let mut rows = Vec::new();
    let mut total = Counts::default();
    for (package, dir) in packages.into_iter().zip(dirs) {
        let counts = walker.walk(&dir)?;
        total.add(&counts);
        let harness = if options.exact { Some(harness_count(&mut cargo, &package.name, limits.harness_output_bytes)?) } else { None };
        rows.push(Row { name: package.name, manifest_path: package.manifest_path, counts, harness });
    }
    Ok(Report { rows, total, exact: options.exact })
}

/// Writes `report` as the `cargo xtask stats` table.
pub fn write_table(report: &Report, out: &mut impl Write) -> io::Result<()> {
    let exact = report.exact;
    let exact_col = if exact { format!(" {:>8}", "harness") } else { String::new() };
    writeln!(out, "{:<26} {:>6} {:>8} {:>7} {:>10}{exact_col}", "crate", "files", "lines", "#[test]", "proptest!")?;
    let rule = "-".repeat(62 + if exact { 9 } else { 0 });
    writeln!(out, "{rule}")?;
    for row in &report.rows {
        let c = &row.counts;
        let ex = match &row.harness {
            None => String::new(),
            Some(Harness::Listed(n)) => format!(" {n:>8}"),
            Some(Harness::Failed(_)) => format!(" {:>8}", "error"),
        };
        writeln!(out, "{:<26} {:>6} {:>8} {:>7} {:>10}{ex}", row.name, c.files, c.lines, c.tests, c.proptests)?;
    }
    writeln!(out, "{rule}")?;
    let t = &report.total;
    let ex = report.harness_total().map_or_else(String::new, |n| format!(" {n:>8}"));
    writeln!(out, "{:<26} {:>6} {:>8} {:>7} {:>10}{ex}", "TOTAL", t.files, t.lines, t.tests, t.proptests)?;
    if exact {
        let failed: Vec<&str> = report.harness_failures().map(|r| r.name.as_str()).collect();
        if !failed.is_empty() {
            writeln!(out, "\n(The harness total leaves out {} crate(s) whose test listing failed: {}.)", failed.len(), failed.join(", "))?;
        }
    } else {
        writeln!(out, "\n(#[test] counts source attributes; macro-generated tests count once. Use --exact for harness counts.)")?;
    }
    Ok(())
}

/// A workspace package from `cargo metadata`.
struct Package {
    name: String,
    manifest_path: PathBuf,
}

/// The workspace members of `cargo metadata` JSON, sorted by name and then by manifest path.
///
/// Packages that are not workspace members (dependencies, when the JSON was made without
/// `--no-deps`) are left out. A member without a package entry, a package listed twice, and a missing
/// or non-string `id`, `name` or `manifest_path` are errors.
fn packages(json: &[u8], limit: usize) -> Result<Vec<Package>, Error> {
    if json.len() > limit {
        return Err(Error::Metadata(format!("{} bytes of JSON is more than the limit of {limit}", json.len())));
    }
    let meta: serde_json::Value = serde_json::from_slice(json).map_err(|e| Error::Metadata(format!("not JSON: {e}")))?;
    let field = |v: &serde_json::Value, what: &str| -> Result<String, Error> {
        v.as_str().map(str::to_owned).ok_or_else(|| Error::Metadata(format!("{what} is missing or not a string")))
    };
    let members = meta["workspace_members"].as_array().ok_or_else(|| Error::Metadata("workspace_members is missing".into()))?;
    let mut wanted = HashSet::new();
    for member in members {
        wanted.insert(field(member, "a workspace_members entry")?);
    }
    let list = meta["packages"].as_array().ok_or_else(|| Error::Metadata("packages is missing".into()))?;
    let mut found = HashSet::new();
    let mut paths = HashSet::new();
    let mut out = Vec::new();
    for p in list {
        let id = field(&p["id"], "a package id")?;
        if !wanted.contains(&id) {
            continue;
        }
        let name = field(&p["name"], &format!("the name of package {id}"))?;
        let manifest_path = PathBuf::from(field(&p["manifest_path"], &format!("the manifest_path of package {id}"))?);
        if !found.insert(id.clone()) || !paths.insert(manifest_path.clone()) {
            return Err(Error::Metadata(format!("package {id} ({}) is listed twice", manifest_path.display())));
        }
        out.push(Package { name, manifest_path });
    }
    if let Some(missing) = wanted.iter().find(|id| !found.contains(*id)) {
        return Err(Error::Metadata(format!("workspace member {missing} has no package entry")));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.manifest_path.cmp(&b.manifest_path)));
    Ok(out)
}

/// Walks crate folders within one run's entry and byte budgets.
struct Walker<'a> {
    limits: &'a Limits,
    /// The folders of every workspace member. A member's folder inside another member's folder is
    /// counted in its own row only.
    members: HashSet<PathBuf>,
    entries: usize,
    bytes: u64,
}

impl Walker<'_> {
    /// Counts the `.rs` files under `root`, without following links.
    fn walk(&mut self, root: &Path) -> Result<Counts, Error> {
        let mut counts = Counts::default();
        let mut pending = vec![(root.to_path_buf(), 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            let io_err = |source| Error::Io { path: dir.clone(), source };
            for entry in std::fs::read_dir(&dir).map_err(io_err)? {
                let entry = entry.map_err(io_err)?;
                if self.entries >= self.limits.entries {
                    return Err(Error::TooManyEntries { path: dir.clone(), limit: self.limits.entries });
                }
                self.entries += 1;
                let path = entry.path();
                // The entry's own type: a link is never followed, so links to a parent folder, or
                // many links to one folder, do not multiply the walk, and a link to a file outside
                // the crate is not counted.
                let kind = entry.file_type().map_err(|source| Error::Io { path: path.clone(), source })?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() {
                    // Nested fuzz workspaces and build output are not part of the crate.
                    if name == "target" || name == "fuzz" || name.starts_with('.') || self.members.contains(&path) {
                        continue;
                    }
                    if depth >= self.limits.depth {
                        return Err(Error::TooDeep { path, limit: self.limits.depth });
                    }
                    pending.push((path, depth + 1));
                } else if kind.is_file() && name.ends_with(".rs") {
                    let src = self.read_source(&path)?;
                    counts.add_source(&src);
                }
            }
        }
        Ok(counts)
    }

    /// A source file's text, read within the per-file and per-run byte limits. A file that grew
    /// past a limit while it is read is an error too, since the read stops one byte past the limit.
    fn read_source(&mut self, path: &Path) -> Result<String, Error> {
        let per_file = self.limits.source_bytes;
        let left = self.limits.total_source_bytes.saturating_sub(self.bytes);
        let cap = per_file.min(left);
        let io_err = |source| Error::Io { path: path.to_path_buf(), source };
        let file = File::open(path).map_err(io_err)?;
        let Some(bytes) = read_bounded(file, cap).map_err(io_err)? else {
            return Err(if per_file <= left {
                Error::SourceTooLarge { path: path.to_path_buf(), limit: per_file }
            } else {
                Error::TotalTooLarge { path: path.to_path_buf(), limit: self.limits.total_source_bytes }
            });
        };
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        String::from_utf8(bytes).map_err(|_| Error::NotUtf8(path.to_path_buf()))
    }
}

/// Everything `reader` gives, or `None` when that is more than `limit` bytes.
fn read_bounded(reader: impl Read, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    reader.take(limit.saturating_add(1)).read_to_end(&mut buf)?;
    Ok((buf.len() as u64 <= limit).then_some(buf))
}

/// `#[test]` attributes and `proptest!` blocks (each block may hold many
/// property tests, and macro-generated tests are only counted once; use
/// `--exact` for harness-accurate numbers).
fn count_tests(src: &str) -> (usize, usize) {
    let mut tests = 0;
    let mut props = 0;
    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with("//") {
            continue;
        }
        tests += t.matches("#[test]").count();
        if t.starts_with("proptest!") {
            props += 1;
        }
    }
    (tests, props)
}

/// The tests a `-- --list` output names: the lines that end in `: test`.
fn count_listed(stdout: &[u8]) -> usize {
    String::from_utf8_lossy(stdout).lines().filter(|l| l.ends_with(": test")).count()
}

/// Runs `cargo test -q -p <package> -- --list` and counts the tests it lists. Output past `limit` is
/// an error; a command that cannot start or fails is a [`Harness::Failed`] row.
fn harness_count(cargo: &mut impl FnMut() -> Command, package: &str, limit: u64) -> Result<Harness, Error> {
    let mut cmd = cargo();
    cmd.args(["test", "-q", "-p", package, "--", "--list"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Ok(Harness::Failed(format!("could not start: {e}"))),
    };
    let read = match child.stdout.take() {
        Some(stdout) => read_bounded(stdout, limit),
        None => Err(io::Error::other("no output pipe")),
    };
    let stdout = match read {
        Ok(Some(stdout)) => stdout,
        Ok(None) => {
            stop(&mut child);
            return Err(Error::HarnessOutputTooLarge { package: package.to_owned(), limit });
        }
        Err(e) => {
            stop(&mut child);
            return Ok(Harness::Failed(format!("reading its output: {e}")));
        }
    };
    match child.wait() {
        Ok(status) if status.success() => Ok(Harness::Listed(count_listed(&stdout))),
        Ok(status) => Ok(Harness::Failed(format!("exited with {status}"))),
        Err(e) => Ok(Harness::Failed(format!("waiting for it: {e}"))),
    }
}

/// Ends a child whose output is no longer read.
fn stop(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::{Counts, Error, Harness, Limits, Options, Report, Row, Walker, collect, count_listed, count_tests, read_bounded, write_table};
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// A new, private directory: `create_dir` fails if the name exists (even as a link), so a
    /// directory somebody else made in advance is never used.
    fn private_dir(tag: &str) -> PathBuf {
        let mut n = 0u32;
        loop {
            let dir = std::env::temp_dir().join(format!("craft-xtask-stats-{tag}-{}-{n}", std::process::id()));
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut b = std::fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
            match b.create(&dir) {
                Ok(()) => return dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("{e}"),
            }
            n += 1;
        }
    }

    fn walk_with(dir: &Path, limits: &Limits) -> Result<Counts, Error> {
        Walker { limits, members: HashSet::new(), entries: 0, bytes: 0 }.walk(dir)
    }

    fn walk(dir: &Path) -> Result<Counts, Error> {
        walk_with(dir, &Limits::default())
    }

    /// `cargo metadata --no-deps` JSON for packages `(id, name, folder)`, all workspace members.
    fn metadata(packages: &[(&str, &str, &Path)]) -> Vec<u8> {
        let list: Vec<_> =
            packages.iter().map(|(id, name, dir)| serde_json::json!({ "id": id, "name": name, "manifest_path": dir.join("Cargo.toml") })).collect();
        let ids: Vec<_> = packages.iter().map(|(id, _, _)| id).collect();
        serde_json::to_vec(&serde_json::json!({ "packages": list, "workspace_members": ids })).unwrap()
    }

    fn no_cargo() -> Command {
        panic!("cargo is only run in exact mode")
    }

    #[test]
    fn counts_attributes_and_proptest_blocks() {
        let src = "#[test]\nfn a() {}\n// #[test] commented\n    #[test]\nfn b() {}\nproptest! {\n}\n";
        assert_eq!(count_tests(src), (2, 1));
    }

    #[cfg(unix)]
    #[test]
    fn a_walk_does_not_follow_directory_links() {
        let dir = private_dir("loop");
        std::fs::write(dir.join("a.rs"), "#[test]\n").unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        let c = walk(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let c = c.unwrap();
        assert_eq!((c.files, c.tests), (1, 1));
    }

    /// A directory alias does not cause its Rust source to be counted twice.
    #[cfg(unix)]
    #[test]
    fn walk_does_not_follow_directory_symlinks() {
        use std::os::unix::fs::symlink;
        let base = private_dir("xtask");
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("a.rs"), "fn a() {}\n").unwrap();
        // A directory symlink pointing back at `real` would be re-scanned if followed.
        symlink(&real, base.join("alias")).unwrap();
        let c = walk(&base).unwrap();
        assert_eq!(c.files, 1, "the aliased directory was followed and counted twice");
        std::fs::remove_dir_all(&base).ok();
    }

    /// Traversal reports an exhausted entry budget and accepts a tree that exactly fits it.
    #[test]
    fn walk_reports_budget_exhaustion() {
        let base = private_dir("stats-budget");
        std::fs::write(base.join("a.rs"), "fn a() {}\n").unwrap();
        let none = walk_with(&base, &Limits { entries: 0, ..Limits::default() });
        let one = walk_with(&base, &Limits { entries: 1, ..Limits::default() });
        std::fs::remove_dir_all(&base).unwrap();
        assert!(matches!(none, Err(Error::TooManyEntries { limit: 0, .. })), "{none:?}");
        assert_eq!(one.unwrap().files, 1);
    }

    #[test]
    fn a_source_past_the_size_limit_is_not_read() {
        let dir = private_dir("size");
        let (small, big) = (dir.join("a.rs"), dir.join("b.rs"));
        std::fs::write(&small, "fn a() {}\n").unwrap();
        std::fs::write(&big, "fn b() { }\n").unwrap();
        let limits = Limits { source_bytes: 10, ..Limits::default() };
        let mut walker = Walker { limits: &limits, members: HashSet::new(), entries: 0, bytes: 0 };
        let small_text = walker.read_source(&small);
        let big_text = walker.read_source(&big);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(small_text.unwrap(), "fn a() {}\n");
        assert!(matches!(big_text, Err(Error::SourceTooLarge { limit: 10, .. })), "{big_text:?}");
    }

    #[cfg(unix)]
    #[test]
    fn walk_counts_regular_sources_once_and_skips_links_and_big_files() {
        let dir = private_dir("walk");
        let sub = dir.join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(dir.join("a.rs"), "#[test]\nfn a() {}\n").unwrap();
        std::fs::write(sub.join("b.rs"), "fn b() {}\n").unwrap();
        // A directory link back to the root and a file link are not followed.
        std::os::unix::fs::symlink(&dir, sub.join("loop")).unwrap();
        std::os::unix::fs::symlink(dir.join("a.rs"), sub.join("link.rs")).unwrap();
        let linked = walk(&dir);
        // A source over the size limit is an error, not a silently smaller count.
        std::fs::write(dir.join("big.rs"), "x".repeat(64)).unwrap();
        let big = walk_with(&dir, &Limits { source_bytes: 32, ..Limits::default() });
        let _ = std::fs::remove_dir_all(dir);
        let c = linked.unwrap();
        assert_eq!((c.files, c.tests), (2, 1));
        assert!(matches!(big, Err(Error::SourceTooLarge { limit: 32, .. })), "{big:?}");
    }

    #[test]
    fn a_source_exactly_at_the_size_limit_is_read() {
        let dir = private_dir("exact-size");
        std::fs::write(dir.join("a.rs"), "fn a() {}\n").unwrap();
        let at = walk_with(&dir, &Limits { source_bytes: 10, ..Limits::default() });
        let under = walk_with(&dir, &Limits { source_bytes: 9, ..Limits::default() });
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(at.unwrap().files, 1);
        assert!(matches!(under, Err(Error::SourceTooLarge { limit: 9, .. })), "{under:?}");
    }

    #[test]
    fn the_total_source_limit_spans_every_file_of_the_run() {
        let dir = private_dir("total");
        std::fs::write(dir.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.join("b.rs"), "fn b() {}\n").unwrap();
        let fits = walk_with(&dir, &Limits { total_source_bytes: 20, ..Limits::default() });
        let over = walk_with(&dir, &Limits { total_source_bytes: 19, ..Limits::default() });
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(fits.unwrap().files, 2);
        assert!(matches!(over, Err(Error::TotalTooLarge { limit: 19, .. })), "{over:?}");
    }

    #[test]
    fn a_folder_past_the_depth_limit_is_an_error() {
        let dir = private_dir("depth");
        std::fs::create_dir_all(dir.join("a").join("b")).unwrap();
        std::fs::write(dir.join("a").join("b").join("c.rs"), "fn c() {}\n").unwrap();
        let fits = walk_with(&dir, &Limits { depth: 2, ..Limits::default() });
        let over = walk_with(&dir, &Limits { depth: 1, ..Limits::default() });
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(fits.unwrap().files, 1);
        assert!(matches!(&over, Err(Error::TooDeep { path, limit: 1 }) if path.ends_with("b")), "{over:?}");
    }

    #[test]
    fn build_output_fuzz_and_hidden_entries_are_skipped() {
        let dir = private_dir("skip");
        for skipped in ["target", "fuzz", ".git"] {
            std::fs::create_dir(dir.join(skipped)).unwrap();
            std::fs::write(dir.join(skipped).join("x.rs"), "#[test]\n").unwrap();
        }
        std::fs::write(dir.join("lib.rs"), "#[test]\n").unwrap();
        std::fs::write(dir.join("notes.rs.txt"), "#[test]\n").unwrap();
        // Only folders are skipped by a leading dot; a file named so is still a source.
        std::fs::write(dir.join(".hidden.rs"), "#[test]\n").unwrap();
        let c = walk(&dir);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(c.unwrap(), Counts { files: 2, lines: 2, tests: 2, proptests: 0 });
    }

    #[test]
    fn a_source_that_is_not_utf8_is_an_error() {
        let dir = private_dir("utf8");
        std::fs::write(dir.join("a.rs"), b"fn a() {}\n\xff\n").unwrap();
        let c = walk(&dir);
        let _ = std::fs::remove_dir_all(dir);
        assert!(matches!(&c, Err(Error::NotUtf8(path)) if path.ends_with("a.rs")), "{c:?}");
    }

    #[test]
    fn a_missing_package_folder_is_an_error() {
        let dir = private_dir("missing");
        let gone = dir.join("gone");
        let json = metadata(&[("gone 0.1.0", "gone", &gone)]);
        let report = collect(&json, &Options::default(), no_cargo);
        let _ = std::fs::remove_dir_all(dir);
        assert!(matches!(&report, Err(Error::Io { path, .. }) if *path == gone), "{report:?}");
    }

    #[test]
    fn metadata_over_its_limit_is_refused_before_it_is_parsed() {
        let options = Options { limits: Limits { metadata_bytes: 4, ..Limits::default() }, ..Options::default() };
        let report = collect(b"not json at all", &options, no_cargo);
        assert!(matches!(&report, Err(Error::Metadata(why)) if why.contains("limit")), "{report:?}");
    }

    #[test]
    fn malformed_metadata_is_an_error() {
        let dir = private_dir("malformed");
        let manifest = dir.join("Cargo.toml");
        let cases = [
            serde_json::json!({ "packages": [] }),
            serde_json::json!({ "workspace_members": [] }),
            serde_json::json!({ "workspace_members": [1], "packages": [] }),
            serde_json::json!({ "workspace_members": ["a"], "packages": [] }),
            serde_json::json!({ "workspace_members": ["a"], "packages": [{ "id": "a", "manifest_path": manifest }] }),
            serde_json::json!({ "workspace_members": ["a"], "packages": [{ "id": "a", "name": "a" }] }),
            serde_json::json!({ "workspace_members": ["a"], "packages": [{ "name": "a", "manifest_path": manifest }] }),
        ];
        let results: Vec<_> = cases.iter().map(|c| collect(&serde_json::to_vec(c).unwrap(), &Options::default(), no_cargo)).collect();
        let not_json = collect(b"{", &Options::default(), no_cargo);
        let _ = std::fs::remove_dir_all(dir);
        for (case, result) in cases.iter().zip(&results) {
            assert!(matches!(result, Err(Error::Metadata(_))), "{case}: {result:?}");
        }
        assert!(matches!(not_json, Err(Error::Metadata(_))), "{not_json:?}");
    }

    #[test]
    fn a_package_listed_twice_is_an_error() {
        let dir = private_dir("twice");
        let twice = metadata(&[("a 0.1.0", "a", &dir), ("a 0.1.0", "a", &dir)]);
        let same_folder = metadata(&[("a 0.1.0", "a", &dir), ("b 0.1.0", "b", &dir)]);
        let twice = collect(&twice, &Options::default(), no_cargo);
        let same_folder = collect(&same_folder, &Options::default(), no_cargo);
        let _ = std::fs::remove_dir_all(dir);
        assert!(matches!(&twice, Err(Error::Metadata(why)) if why.contains("twice")), "{twice:?}");
        assert!(matches!(&same_folder, Err(Error::Metadata(why)) if why.contains("twice")), "{same_folder:?}");
    }

    #[test]
    fn packages_outside_the_workspace_are_not_walked() {
        let dir = private_dir("deps");
        std::fs::write(dir.join("lib.rs"), "#[test]\n").unwrap();
        let dependency = dir.join("not-here");
        let json = serde_json::to_vec(&serde_json::json!({
            "workspace_members": ["a 0.1.0"],
            "packages": [
                { "id": "dep 1.0.0", "name": "dep", "manifest_path": dependency.join("Cargo.toml") },
                { "id": "a 0.1.0", "name": "a", "manifest_path": dir.join("Cargo.toml") },
            ],
        }))
        .unwrap();
        let report = collect(&json, &Options::default(), no_cargo);
        let _ = std::fs::remove_dir_all(dir);
        let report = report.unwrap();
        assert_eq!(report.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(report.total.tests, 1);
    }

    #[test]
    fn rows_sort_by_name_and_the_total_adds_them() {
        let dir = private_dir("order");
        for (name, src) in [("zeta", "#[test]\n#[test]\n"), ("alpha", "proptest! {\n}\n")] {
            std::fs::create_dir(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("lib.rs"), src).unwrap();
        }
        let json = metadata(&[("z 0.1.0", "zeta", &dir.join("zeta")), ("a 0.1.0", "alpha", &dir.join("alpha"))]);
        let report = collect(&json, &Options::default(), no_cargo);
        let _ = std::fs::remove_dir_all(dir);
        let report = report.unwrap();
        assert_eq!(report.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["alpha", "zeta"]);
        assert_eq!(report.total, Counts { files: 2, lines: 4, tests: 2, proptests: 1 });
        assert_eq!(report.harness_total(), None);
    }

    #[test]
    fn a_nested_workspace_member_is_counted_in_its_own_row_only() {
        let dir = private_dir("nested");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(dir.join("xtask").join("src")).unwrap();
        std::fs::write(dir.join("xtask").join("src").join("main.rs"), "#[test]\nfn t() {}\n").unwrap();
        let json = metadata(&[("app 0.1.0", "app", &dir), ("xtask 0.1.0", "xtask", &dir.join("xtask"))]);
        let report = collect(&json, &Options::default(), no_cargo);
        let _ = std::fs::remove_dir_all(dir);
        let report = report.unwrap();
        let rows: Vec<_> = report.rows.iter().map(|r| (r.name.as_str(), r.counts.files, r.counts.tests)).collect();
        assert_eq!(rows, [("app", 1, 0), ("xtask", 1, 1)]);
        assert_eq!((report.total.files, report.total.tests), (2, 1));
    }

    fn row(name: &str, counts: Counts, harness: Option<Harness>) -> Row {
        Row { name: name.into(), manifest_path: PathBuf::from(name).join("Cargo.toml"), counts, harness }
    }

    fn table(report: &Report) -> String {
        let mut out = Vec::new();
        write_table(report, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_table_keeps_its_columns() {
        let counts = Counts { files: 3, lines: 120, tests: 7, proptests: 1 };
        let report = Report { rows: vec![row("app-core", counts, None)], total: counts, exact: false };
        let expected = concat!(
            "crate                       files    lines #[test]  proptest!\n",
            "--------------------------------------------------------------\n",
            "app-core                        3      120       7          1\n",
            "--------------------------------------------------------------\n",
            "TOTAL                           3      120       7          1\n",
            "\n(#[test] counts source attributes; macro-generated tests count once. Use --exact for harness counts.)\n",
        );
        assert_eq!(table(&report), expected);
    }

    #[test]
    fn the_exact_table_marks_failed_listings_and_leaves_them_out_of_the_total() {
        let counts = Counts { files: 1, lines: 2, tests: 1, proptests: 0 };
        let report = Report {
            rows: vec![row("a", counts, Some(Harness::Listed(4))), row("b", counts, Some(Harness::Failed("exited with 101".into())))],
            total: Counts { files: 2, lines: 4, tests: 2, proptests: 0 },
            exact: true,
        };
        let expected = concat!(
            "crate                       files    lines #[test]  proptest!  harness\n",
            "-----------------------------------------------------------------------\n",
            "a                               1        2       1          0        4\n",
            "b                               1        2       1          0    error\n",
            "-----------------------------------------------------------------------\n",
            "TOTAL                           2        4       2          0        4\n",
            "\n(The harness total leaves out 1 crate(s) whose test listing failed: b.)\n",
        );
        assert_eq!(table(&report), expected);
        assert_eq!(report.harness_total(), Some(4));
        assert_eq!(report.harness_failures().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn listed_tests_are_the_lines_that_end_in_test() {
        assert_eq!(count_listed(b"a::one: test\na::two: test\nbench: benchmark\n\n2 tests, 1 benchmark\n"), 2);
        assert_eq!(count_listed(b""), 0);
    }

    #[test]
    fn a_bounded_read_refuses_more_than_its_limit() {
        assert_eq!(read_bounded(&b"abcd"[..], 4).unwrap().as_deref(), Some(&b"abcd"[..]));
        assert_eq!(read_bounded(&b"abcde"[..], 4).unwrap(), None);
        assert_eq!(read_bounded(&b""[..], 0).unwrap().as_deref(), Some(&b""[..]));
    }

    /// A stand-in for `cargo`: `sh -c <script>` gets `test -q -p <crate> -- --list` as `$1` to `$6`.
    #[cfg(unix)]
    fn fake_cargo(script: &'static str) -> impl FnMut() -> Command {
        move || {
            let mut c = Command::new("sh");
            c.args(["-c", script, "sh"]);
            c
        }
    }

    #[cfg(unix)]
    #[test]
    fn exact_mode_counts_listed_tests_and_marks_failures() {
        let _guard = crate::spawn_guard();
        let dir = private_dir("exact");
        for name in ["good", "bad"] {
            std::fs::create_dir(dir.join(name)).unwrap();
        }
        let json = metadata(&[("good 0.1.0", "good", &dir.join("good")), ("bad 0.1.0", "bad", &dir.join("bad"))]);
        let script = r#"[ "$1 $2 $3 $5 $6" = "test -q -p -- --list" ] || exit 9
case "$4" in good) printf 'x: test\ny: test\nz: benchmark\n' ;; *) exit 101 ;; esac"#;
        let options = Options { exact: true, ..Options::default() };
        let report = collect(&json, &options, fake_cargo(script));
        let _ = std::fs::remove_dir_all(dir);
        let report = report.unwrap();
        let harness: Vec<_> = report.rows.iter().map(|r| (r.name.as_str(), r.harness.clone())).collect();
        assert_eq!(harness[1], ("good", Some(Harness::Listed(2))));
        assert_eq!(harness[0].0, "bad");
        assert!(matches!(&harness[0].1, Some(Harness::Failed(why)) if why.contains("101")), "{harness:?}");
        assert_eq!(report.harness_total(), Some(2));
    }

    #[cfg(unix)]
    #[test]
    fn exact_mode_output_past_its_limit_is_an_error() {
        let _guard = crate::spawn_guard();
        let dir = private_dir("exact-limit");
        let json = metadata(&[("a 0.1.0", "a", &dir)]);
        let options = Options { exact: true, limits: Limits { harness_output_bytes: 16, ..Limits::default() } };
        let report = collect(&json, &options, fake_cargo("yes 'one: test'"));
        let _ = std::fs::remove_dir_all(dir);
        assert!(matches!(&report, Err(Error::HarnessOutputTooLarge { package, limit: 16 }) if package == "a"), "{report:?}");
    }

    #[test]
    fn exact_mode_marks_a_command_that_cannot_start() {
        let _guard = crate::spawn_guard();
        let dir = private_dir("exact-spawn");
        let json = metadata(&[("a 0.1.0", "a", &dir)]);
        let options = Options { exact: true, ..Options::default() };
        let report = collect(&json, &options, || Command::new(dir.join("no-such-program")));
        let _ = std::fs::remove_dir_all(&dir);
        let report = report.unwrap();
        assert!(matches!(&report.rows[0].harness, Some(Harness::Failed(why)) if why.starts_with("could not start")), "{report:?}");
        assert_eq!(report.harness_total(), Some(0));
    }
}
