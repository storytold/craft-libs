#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Embed the optional [craft-i18n](https://github.com/storytold/craft-i18n) translation catalogs
//! from an app's `build.rs`, as the `CRAFT_I18N` static that `craft_i18n::I18n::new` takes.
//!
//! Apps never depend on craft-i18n in `Cargo.toml` (Cargo would fetch the catalog repository for
//! every build). The build reads a local checkout named by `CRAFT_I18N_DIR` instead:
//!
//! - **Unset or empty:** only the app's own catalogs ([`Catalogs::app_dir`]) are embedded, with no
//!   shared catalog; without them the table is empty and the app is English only.
//! - **Set:** every locale with the app's catalog (`locales/<locale>/<app>.tsv`, here or in
//!   [`Catalogs::app_dir`]) is embedded with its shared catalog (`common.tsv`), and so is the
//!   locale it falls back to (`pt` for `pt-br`), whose catalogs `pt-br` reads when it has no row.
//! - **Set, but not an absolute path to a checkout, or a catalog in it can't be read:** a
//!   `cargo::warning`, and the checkout is not used.
//! - **`CRAFT_I18N_REQUIRED=1`** (release builds; any other value is ignored): an unset, relative
//!   or unreadable checkout is an [`Error`].
//!
//! Add this crate as a build-dependency. In `build.rs`'s `main`:
//!
//! ```ignore
//! println!("cargo::rerun-if-changed=build.rs");
//! match craft_i18n_build::Catalogs::new("wordcraft").write("craft_i18n.rs") {
//!     Ok(locales) => { /* locales.iter().flat_map(|l| l.files()) for craft-fonts-build's subset_to */ }
//!     Err(e) => println!("cargo::error={e}"),
//! }
//! ```
//!
//! and in the crate:
//!
//! ```ignore
//! include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));
//!
//! let (i18n, problems) = craft_i18n::I18n::new(CRAFT_I18N, CRAFT_I18N_OFFERED);
//! ```
//!
//! The generated file defines `CRAFT_I18N: &[(&str, &str, &str)]` (`craft_i18n::Embedded`: locale,
//! shared catalog, app catalog, `""` when missing) and `CRAFT_I18N_OFFERED: &[&str]`, the locales
//! with the app's own catalog, for `I18n::new`'s offered list. Each catalog is embedded with
//! `include_str!` on its path as given, so `CRAFT_I18N_DIR` must be absolute and is never
//! canonicalized (on Windows that gives `\\?\C:\…`, which `include_str!` can't read).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::fmt::{self, Write as _};
use std::io;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

const DIR_VAR: &str = "CRAFT_I18N_DIR";
const REQUIRED_VAR: &str = "CRAFT_I18N_REQUIRED";
/// The catalog the Craft apps share, in each locale folder.
const SHARED: &str = "common.tsv";
/// The largest catalog file read, in bytes.
pub const MAX_CATALOG_BYTES: u64 = 8 << 20;

/// Which catalogs to embed, and from where. [`Catalogs::write`] does the work.
pub struct Catalogs {
    app: String,
    app_dir: Option<PathBuf>,
    dir: Option<PathBuf>,
    required: Option<bool>,
}

/// One embedded locale: its lowercase code and the catalogs it embeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locale {
    /// Lowercase BCP 47 code, as the folder is named (`pt-br`).
    pub code: String,
    /// The checkout's `common.tsv`, if it has one.
    pub shared: Option<PathBuf>,
    /// The app's catalog: the checkout's, the app's own ([`Catalogs::app_dir`]), or the two merged
    /// into a file under `OUT_DIR`.
    pub app: Option<PathBuf>,
}

impl Locale {
    /// Whether the app has its own catalog for this locale, so it is in `CRAFT_I18N_OFFERED`. A
    /// locale without one is embedded only as the fallback of a regional locale.
    pub fn offered(&self) -> bool {
        self.app.is_some()
    }

    /// The catalog files embedded for this locale.
    pub fn files(&self) -> impl Iterator<Item = &Path> {
        [&self.shared, &self.app].into_iter().flatten().map(PathBuf::as_path)
    }
}

/// The environment [`Catalogs::run`] reads, passed in so tests don't set process variables.
pub(crate) struct Env {
    pub dir: Option<OsString>,
    pub required: Option<OsString>,
    pub out_dir: Option<OsString>,
    pub manifest_dir: Option<OsString>,
}

/// A locale folder's catalog files, when present.
#[derive(Default)]
struct Found {
    shared: Option<PathBuf>,
    app: Option<PathBuf>,
}

impl Catalogs {
    /// The catalogs of the app named `app` (the `<app>.tsv` file name: lowercase letters, digits
    /// and `-`), from `CRAFT_I18N_DIR`, required when `CRAFT_I18N_REQUIRED=1`.
    pub fn new(app: &str) -> Self {
        Self { app: app.to_owned(), app_dir: None, dir: None, required: None }
    }

    /// Also embed the app's own catalogs, `<dir>/<locale>/<app>.tsv`, for an app that keeps some
    /// in its repository. A locale here is offered even when the checkout lacks it. When the
    /// checkout has the same locale's app catalog too, the two are merged into one file under
    /// `OUT_DIR`: the rows here, then the checkout's rows whose context and source text are not
    /// defined here by a row craft-i18n accepts; comments and blank lines are left out. A relative
    /// `dir` is under the package directory. Unlike the checkout, a catalog here that can't be
    /// read is always an [`Error`].
    pub fn app_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.app_dir = Some(dir.into());
        self
    }

    /// Read this checkout instead of `CRAFT_I18N_DIR`.
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Override `CRAFT_I18N_REQUIRED` (required only when it is exactly `1`).
    pub fn required(mut self, required: bool) -> Self {
        self.required = Some(required);
        self
    }

    /// Write the catalog table to `out_file` (relative to `OUT_DIR` unless absolute) and print the
    /// `cargo::` lines; returns the embedded locales.
    ///
    /// The table is written in every case: on an error it is empty, and the build fails only on
    /// the message the caller prints. A file is rewritten only when its bytes change, so an
    /// unchanged checkout does not rebuild the crate that embeds it.
    pub fn write(&self, out_file: impl AsRef<Path>) -> Result<Vec<Locale>, Error> {
        let env = Env {
            dir: std::env::var_os(DIR_VAR),
            required: std::env::var_os(REQUIRED_VAR),
            out_dir: std::env::var_os("OUT_DIR"),
            manifest_dir: std::env::var_os("CARGO_MANIFEST_DIR"),
        };
        let mut cargo = Vec::new();
        let result = self.run(&env, out_file.as_ref(), &mut cargo);
        for line in cargo {
            println!("{line}");
        }
        result
    }

    pub(crate) fn run(&self, env: &Env, out_file: &Path, cargo: &mut Vec<String>) -> Result<Vec<Locale>, Error> {
        cargo.push(format!("cargo::rerun-if-env-changed={DIR_VAR}"));
        cargo.push(format!("cargo::rerun-if-env-changed={REQUIRED_VAR}"));
        let out = match (out_file.is_absolute(), &env.out_dir) {
            (true, _) => out_file.to_path_buf(),
            (false, Some(out_dir)) => Path::new(out_dir).join(out_file),
            (false, None) => return Err(Error::NoOutDir),
        };
        match self.locales(env, &out, cargo) {
            Ok(locales) => {
                write_if_changed(&out, &source(&locales))?;
                Ok(locales)
            }
            Err(e) => {
                write_if_changed(&out, &source(&[]))?;
                Err(e)
            }
        }
    }

    /// The locales to embed, with merged app catalogs written beside `out`.
    fn locales(&self, env: &Env, out: &Path, cargo: &mut Vec<String>) -> Result<Vec<Locale>, Error> {
        if self.app.is_empty() || !self.app.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return Err(Error::AppName { app: self.app.clone() });
        }
        let app_file = format!("{}.tsv", self.app);
        let tree = match &self.app_dir {
            Some(dir) => {
                let dir = match (dir.is_absolute(), &env.manifest_dir) {
                    (false, Some(manifest)) => Path::new(manifest).join(dir),
                    _ => dir.clone(),
                };
                tree_locales(&dir, &app_file, cargo)?
            }
            None => BTreeMap::new(),
        };
        let required = self.required.unwrap_or_else(|| env.required.as_ref().is_some_and(|v| v == "1"));
        let dir = self.dir.clone().or_else(|| env.dir.clone().filter(|d| !d.is_empty()).map(PathBuf::from));
        let checkout = match dir {
            None if required => return Err(Error::NotSet),
            None => BTreeMap::new(),
            Some(dir) => {
                // The watches stay when the checkout is rejected, so repairing it reruns the build.
                match checkout_locales(&dir, &app_file, cargo) {
                    Ok(found) => found,
                    Err(e) if required => return Err(e),
                    Err(e) => {
                        let shown = e.to_string().replace(['\n', '\r'], " ");
                        cargo.push(format!("cargo::warning=building without the craft-i18n catalogs: {shown}"));
                        BTreeMap::new()
                    }
                }
            }
        };
        combine(&tree, &checkout, &app_file, out.parent().unwrap_or(Path::new("")))
    }
}

/// The locales to embed from the app's own catalogs (`tree`) and the checkout's, both keyed by
/// lowercase code: every locale with an app catalog, and the locale each one falls back to when
/// either has that folder.
fn combine(tree: &BTreeMap<String, PathBuf>, checkout: &BTreeMap<String, Found>, app_file: &str, out_dir: &Path) -> Result<Vec<Locale>, Error> {
    let offered: BTreeSet<&str> =
        tree.keys().map(String::as_str).chain(checkout.iter().filter(|(_, f)| f.app.is_some()).map(|(c, _)| c.as_str())).collect();
    let mut codes = offered.clone();
    for code in &offered {
        if let Some(parent) = region_parent(code)
            && (tree.contains_key(parent) || checkout.contains_key(parent))
        {
            codes.insert(parent);
        }
    }
    let mut locales = Vec::new();
    for code in codes {
        let found = checkout.get(code);
        let theirs = found.and_then(|f| f.app.as_ref());
        let app = match (tree.get(code), theirs) {
            (Some(ours), Some(theirs)) => {
                let path = out_dir.join("craft-i18n").join(code).join(app_file);
                let text = merge(&read_catalog(ours)?, &read_catalog(theirs)?);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|source| Error::Write { path: parent.to_path_buf(), source })?;
                }
                write_if_changed(&path, &text)?;
                Some(utf8(path)?)
            }
            (ours, theirs) => ours.or(theirs).cloned(),
        };
        locales.push(Locale { code: code.to_owned(), shared: found.and_then(|f| f.shared.clone()), app });
    }
    Ok(locales)
}

/// The parent a regional locale falls back to (`pt` for `pt-br`, `zh-hant` for `zh-hant-hk`): the
/// code without its last subtag when that is a region (two letters or three digits), as
/// craft-i18n's lookup falls back.
fn region_parent(code: &str) -> Option<&str> {
    let (parent, region) = code.rsplit_once('-')?;
    let is_region =
        (region.len() == 2 && region.bytes().all(|b| b.is_ascii_alphabetic())) || (region.len() == 3 && region.bytes().all(|b| b.is_ascii_digit()));
    is_region.then_some(parent)
}

/// The rows of `ours`, then every row of `theirs` whose context and source text `ours` doesn't
/// define with a row craft-i18n accepts. Comments and blank lines are left out. The columns are
/// compared as written: each text has one spelling in the catalog format, whose only escapes are
/// `\n`, `\t` and `\\`. A row of `ours` that craft-i18n's parser (`crates/i18n/src/catalog.rs`)
/// skips defines nothing, so the checkout's row stays and the app's row is skipped and reported at
/// run time.
fn merge(ours: &str, theirs: &str) -> String {
    fn rows(text: &str) -> impl Iterator<Item = &str> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).filter(|l| !l.is_empty() && !l.starts_with('#'))
    }
    fn key(line: &str) -> Option<(&str, &str)> {
        let mut columns = line.splitn(3, '\t');
        Some((columns.next()?, columns.next()?))
    }
    let defined: HashMap<(&str, &str), Option<usize>> = rows(ours).filter_map(accepted).collect();
    let hidden = |line: &&str| {
        key(line).is_some_and(|k| {
            defined.get(&k).is_some_and(|ours| {
                // The build doesn't know a locale's plural forms; the checkout's row has the right
                // count, so an `@plural` row of `ours` with another count is one craft-i18n skips.
                ours.is_none() || *ours == line.rsplit_once('\t').and_then(|(_, translation)| plural_forms(k.0, translation))
            })
        })
    };
    let mut out = String::new();
    for line in rows(ours).chain(rows(theirs).filter(|l| !hidden(l))) {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The context and source text of `line` as written, with the number of plural forms of an
/// `@plural` row, when craft-i18n's parser (`crates/i18n/src/catalog.rs`) accepts the row: three
/// columns, no invisible character, valid escapes, a source text and a translation, and the checks
/// of its context. The plural form count is left to the caller, which knows no locale's rules.
fn accepted(line: &str) -> Option<((&str, &str), Option<usize>)> {
    fn escapes_valid(text: &str) -> bool {
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\\' && !matches!(chars.next(), Some('n' | 't' | '\\')) {
                return false;
            }
        }
        true
    }
    /// `{name}` placeholders, sorted, as craft-i18n compares them.
    fn placeholders(text: &str) -> Vec<&str> {
        let mut names = Vec::new();
        let mut rest = text;
        while let Some((_, after)) = rest.split_once('{') {
            let Some((name, after)) = after.split_once('}') else {
                break;
            };
            names.push(name);
            rest = after;
        }
        names.sort_unstable();
        names
    }
    let [context, source, translation] = line.split('\t').collect::<Vec<_>>()[..] else {
        return None;
    };
    if line.chars().any(|c| c != '\t' && (c.is_control() || c == '\u{feff}')) {
        return None;
    }
    if ![context, source, translation].into_iter().all(escapes_valid) || source.is_empty() || translation.is_empty() {
        return None;
    }
    // Escapes never produce `{`, `}` or `…`, so the checks below read the columns as written.
    let valid = match context {
        "@id" => true,
        "@plural" => {
            let english: Vec<&str> = source.split('|').collect();
            let translated: Vec<&str> = translation.split('|').collect();
            match english[..] {
                [one, other] => {
                    !one.is_empty()
                        && !other.is_empty()
                        && placeholders(one) == placeholders(other)
                        && translated.iter().all(|f| !f.is_empty() && placeholders(f) == placeholders(other))
                }
                _ => false,
            }
        }
        c if c.starts_with('@') && c != "@msg" => false,
        _ => placeholders(source) == placeholders(translation) && source.ends_with('…') == translation.ends_with('…'),
    };
    valid.then(|| ((context, source), plural_forms(context, translation)))
}

/// The number of plural forms of an `@plural` row's translation, `None` for other rows.
fn plural_forms(context: &str, translation: &str) -> Option<usize> {
    (context == "@plural").then(|| translation.split('|').count())
}

/// The locale folders of `locales`, by lowercase code, without `en` (the source text), `pseudo`
/// and names starting with `.`.
fn locale_dirs(locales: &Path, cargo: &mut Vec<String>) -> Result<BTreeMap<String, PathBuf>, Error> {
    let entries = std::fs::read_dir(locales).map_err(|source| Error::Read { path: locales.to_path_buf(), source })?;
    let mut dirs = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::Read { path: locales.to_path_buf(), source })?;
        let path = entry.path();
        if entry.file_name().as_encoded_bytes().starts_with(b".") {
            continue;
        }
        // `Path::is_dir` reads an error as "not a folder", which would drop a locale unreported.
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => continue,
            Err(source) => return Err(Error::Read { path, source }),
        }
        let Some(code) = entry.file_name().to_str().map(str::to_ascii_lowercase) else {
            cargo.push(format!("cargo::warning=craft-i18n: {} is not a UTF-8 name and is not embedded", path.display()).replace(['\n', '\r'], " "));
            continue;
        };
        if code == "en" || code == "pseudo" {
            continue;
        }
        if dirs.insert(code.clone(), path).is_some() {
            return Err(Error::LocaleTwice { locales: locales.to_path_buf(), code });
        }
    }
    Ok(dirs)
}

/// The app's own catalogs: the `<app>.tsv` of each locale folder under `dir`, each checked.
fn tree_locales(dir: &Path, app_file: &str, cargo: &mut Vec<String>) -> Result<BTreeMap<String, PathBuf>, Error> {
    rerun_if_changed(cargo, dir)?;
    let mut tree = BTreeMap::new();
    for (code, locale) in locale_dirs(dir, cargo)? {
        if let Some(path) = catalog_file(locale.join(app_file))? {
            tree.insert(code, path);
        }
    }
    Ok(tree)
}

/// `path` if a catalog is there, checked by [`read_catalog`]; `None` if nothing is. Anything else
/// by that name (a folder, a FIFO) is an [`Error::Read`], so a locale doesn't lose its rows
/// unreported.
fn catalog_file(path: PathBuf) -> Result<Option<PathBuf>, Error> {
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Read { path, source }),
        Ok(_) => {
            read_catalog(&path)?;
            utf8(path).map(Some)
        }
    }
}

/// The catalogs of the checkout at `dir`, by lowercase code, each checked.
fn checkout_locales(dir: &Path, app_file: &str, cargo: &mut Vec<String>) -> Result<BTreeMap<String, Found>, Error> {
    // `include_str!` resolves a relative path against the generated file.
    if !dir.is_absolute() {
        return Err(Error::NotAbsolute { path: dir.to_path_buf() });
    }
    let locales = dir.join("locales");
    if !locales.is_dir() {
        // Not watched: Cargo scans a watched folder's whole tree on every build, and a folder
        // without `locales/` may be any folder (a home directory), while a missing watched path
        // counts as changed and reruns every build.
        return Err(Error::NoLocales { path: dir.to_path_buf() });
    }
    rerun_if_changed(cargo, &locales)?;
    let mut found = BTreeMap::new();
    for (code, locale) in locale_dirs(&locales, cargo)? {
        let (shared, app) = (catalog_file(locale.join(SHARED))?, catalog_file(locale.join(app_file))?);
        if shared.is_some() || app.is_some() {
            found.insert(code, Found { shared, app });
        }
    }
    Ok(found)
}

/// A catalog's text: a regular file of UTF-8, at most [`MAX_CATALOG_BYTES`], so a device, a FIFO
/// or a file that grows while it is read can't fill memory or block the build.
fn read_catalog(path: &Path) -> Result<String, Error> {
    use std::io::Read as _;
    let read = || -> io::Result<String> {
        // Opening a FIFO with no writer blocks, so refuse anything that isn't a file before opening.
        if !std::fs::metadata(path)?.is_file() {
            return Err(io::Error::other("not a regular file"));
        }
        let file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.take(MAX_CATALOG_BYTES.saturating_add(1)).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_CATALOG_BYTES {
            return Err(io::Error::other(format!("larger than {MAX_CATALOG_BYTES} bytes")));
        }
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    };
    read().map_err(|source| Error::Read { path: path.to_path_buf(), source })
}

/// `path`, which `include_str!` needs as a UTF-8 string.
fn utf8(path: PathBuf) -> Result<PathBuf, Error> {
    if path.to_str().is_none() { Err(Error::NonUtf8Path { path }) } else { Ok(path) }
}

/// Print `cargo::rerun-if-changed` for `path`, a folder Cargo scans for changes. Cargo reads one
/// instruction a line, so a path with a line break would print a second one.
fn rerun_if_changed(cargo: &mut Vec<String>, path: &Path) -> Result<(), Error> {
    let shown = path.display().to_string();
    if shown.contains(['\n', '\r']) {
        return Err(Error::LineBreakInPath { path: path.to_path_buf() });
    }
    cargo.push(format!("cargo::rerun-if-changed={shown}"));
    Ok(())
}

/// The generated Rust: `CRAFT_I18N` and `CRAFT_I18N_OFFERED`.
fn source(locales: &[Locale]) -> String {
    let include = |path: &Option<PathBuf>| match path.as_ref().and_then(|p| p.to_str()) {
        // `{:?}` escapes the path (Windows backslashes) as a Rust string literal.
        Some(p) => format!("include_str!({p:?})"),
        None => "\"\"".to_owned(),
    };
    let mut o = String::from("// Generated by craft-i18n-build from CRAFT_I18N_DIR and the app's catalogs. Do not edit.\n");
    o.push_str("/// The embedded catalogs: (locale, shared catalog, the app's catalog); \"\" for a missing one.\n");
    o.push_str("pub static CRAFT_I18N: &[(&str, &str, &str)] = &[\n");
    for l in locales {
        let _ = writeln!(o, "    ({:?}, {}, {}),", l.code, include(&l.shared), include(&l.app));
    }
    o.push_str("];\n");
    let offered: Vec<&str> = locales.iter().filter(|l| l.offered()).map(|l| l.code.as_str()).collect();
    o.push_str("/// The locales with the app's own catalog: the ones to offer.\n");
    let _ = writeln!(o, "pub static CRAFT_I18N_OFFERED: &[&str] = &{offered:?};");
    o
}

/// Write `text` to `path` unless it already holds exactly that, so Cargo doesn't rebuild what
/// includes it.
fn write_if_changed(path: &Path, text: &str) -> Result<(), Error> {
    if std::fs::read(path).ok().as_deref() == Some(text.as_bytes()) {
        return Ok(());
    }
    std::fs::write(path, text).map_err(|source| Error::Write { path: path.to_path_buf(), source })
}

/// Why the catalogs can't be embedded.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// `CRAFT_I18N_REQUIRED=1` but `CRAFT_I18N_DIR` is unset or empty.
    NotSet,
    /// The app name is empty or has a character other than a lowercase letter, a digit or `-`.
    AppName { app: String },
    /// `CRAFT_I18N_DIR` is a relative path.
    NotAbsolute { path: PathBuf },
    /// `CRAFT_I18N_DIR` has no `locales/` folder.
    NoLocales { path: PathBuf },
    /// A folder or catalog can't be read, isn't a regular file, isn't UTF-8 or is larger than
    /// [`MAX_CATALOG_BYTES`].
    Read { path: PathBuf, source: io::Error },
    /// Two locale folders differ only in case.
    LocaleTwice { locales: PathBuf, code: String },
    /// A catalog to embed has a path that isn't UTF-8, which `include_str!` can't name.
    NonUtf8Path { path: PathBuf },
    /// A path that goes into a `cargo::rerun-if-changed` line has a line break, which would start
    /// another cargo instruction.
    LineBreakInPath { path: PathBuf },
    /// The generated table or a merged catalog can't be written.
    Write { path: PathBuf, source: io::Error },
    /// `out_file` is relative and `OUT_DIR` is unset (not running as a build script).
    NoOutDir,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSet => {
                write!(f, "{REQUIRED_VAR}=1 but {DIR_VAR} is not set: set it to the absolute path of a storytold/craft-i18n checkout")
            }
            Self::AppName { app } => write!(f, "{app:?} is not a catalog name: use lowercase letters, digits and -"),
            Self::NotAbsolute { path } => write!(f, "{DIR_VAR} must be an absolute path, not {}", path.display()),
            Self::NoLocales { path } => write!(
                f,
                "{} has no locales/ folder: {DIR_VAR} must name a storytold/craft-i18n checkout; the build does not watch for the folder, so after \
                 adding it, change build.rs or run cargo clean -p on the package to rebuild",
                path.display()
            ),
            Self::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::LocaleTwice { locales, code } => write!(f, "{} has the locale {code} twice, in different cases", locales.display()),
            Self::NonUtf8Path { path } => write!(f, "cannot embed {}: include_str! needs a UTF-8 path", path.display()),
            Self::LineBreakInPath { path } => write!(f, "{:?} has a line break, which a cargo:: instruction can't hold", path.display().to_string()),
            Self::Write { path, source } => write!(f, "cannot write {}: {source}", path.display()),
            Self::NoOutDir => write!(f, "OUT_DIR is not set: call craft-i18n-build from a build script, or pass an absolute out_file"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}
