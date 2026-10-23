//! The build-script side: read `CRAFT_FONTS_DIR`, select faces, write the generated table.

#[cfg(feature = "subset")]
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt::{self, Write as _};
use std::io;
use std::path::{Path, PathBuf};

use crate::cmap::Cmap;
use crate::manifest::{ManifestEntry, ParseError, parse_manifest, script_matches};

const DIR_VAR: &str = "CRAFT_FONTS_DIR";
const REQUIRED_VAR: &str = "CRAFT_FONTS_REQUIRED";
const EMPTY_TABLE: &str = "&[]\n";

/// Whether the build script is building for wasm32 (web builds usually embed fewer faces).
pub fn target_is_wasm() -> bool {
    std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|arch| arch == "wasm32")
}

type Filter = Box<dyn Fn(&ManifestEntry) -> bool>;

/// Which faces to embed, and from where. [`Embed::write`] does the work.
#[derive(Default)]
pub struct Embed {
    scripts: Option<Vec<String>>,
    filter: Option<Filter>,
    dir: Option<PathBuf>,
    required: Option<bool>,
    ui: bool,
    catalogs: Option<Vec<PathBuf>>,
    texts: Vec<PathBuf>,
    families: Option<Vec<String>>,
    elsewhere: Vec<String>,
    serve: Option<Serve>,
}

/// Where [`Embed::serve`] copies the faces, and the URL prefix they are served under.
struct Serve {
    dir: PathBuf,
    url_prefix: String,
}

/// A selected face and the file the table embeds it from.
struct Face {
    entry: ManifestEntry,
    path: PathBuf,
    index: u32,
    /// An interface face to cut to the catalogs' characters before it is embedded.
    cut: bool,
    /// Hex SHA-256 of the file: the manifest's, or the cut's.
    sha256: String,
}

impl Embed {
    /// Every face except the interface faces' sources (`role` `ui`), from `CRAFT_FONTS_DIR`,
    /// required when `CRAFT_FONTS_REQUIRED=1`. Narrow it with [`Embed::scripts`] and
    /// [`Embed::filter`]; a table of document fonts starts from [`Embed::documents`] instead.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every face of these families, whole, and nothing else: an app's document fonts. A family
    /// is named as the manifest spells it (`"Noto Sans"`, `"Shippori Mincho"`) and contributes all
    /// its styles, interface faces included (`"BIZ UDPGothic"` brings its Regular as well).
    /// A name the manifest lacks is an [`Error::UnknownFamilies`], so the build fails with
    /// `CRAFT_FONTS_REQUIRED=1` and warns and embeds nothing otherwise. Narrow it further with
    /// [`Embed::scripts`] and [`Embed::filter`].
    pub fn documents(families: &[&str]) -> Self {
        Self { families: Some(families.iter().map(|f| (*f).to_owned()).collect()), ..Self::default() }
    }

    /// Embed only faces with at least one of these script tags (see
    /// [`ManifestEntry::has_script`]). With `CRAFT_FONTS_REQUIRED=1`, each tag must match a face.
    pub fn scripts(mut self, tags: &[&str]) -> Self {
        self.scripts = Some(tags.iter().map(|t| (*t).to_owned()).collect());
        self
    }

    /// Embed only faces for which `keep` returns true (applied after [`Embed::scripts`]).
    pub fn filter(mut self, keep: impl Fn(&ManifestEntry) -> bool + 'static) -> Self {
        self.filter = Some(Box::new(keep));
        self
    }

    /// Embed faces for an app's interface: CJK faces only as the interface faces (`role` `ui` in
    /// the manifest), cut to the characters of the catalogs given to `Embed::subset_to`. Other
    /// CJK faces are dropped and the rest are kept. Without catalogs the interface faces are left
    /// out too, with a `cargo::warning`, so a full CJK face is never embedded for the interface.
    /// A manifest with no interface face is also a `cargo::warning` when that drops a CJK face.
    /// Combines with [`Embed::scripts`], [`Embed::filter`] and [`Embed::serve`]; with
    /// `CRAFT_FONTS_REQUIRED=1`, an interface face satisfies its script tags.
    pub fn for_ui(mut self) -> Self {
        self.ui = true;
        self
    }

    /// Cut the interface faces to the characters these translation catalogs use, and embed the
    /// cut faces, written to `OUT_DIR`. Implies [`Embed::for_ui`]. An empty list counts as no
    /// catalogs: the interface faces are left out, with a `cargo::warning`.
    ///
    /// The cut keeps the catalogs' characters from U+2E80 up (CJK and later blocks, without emoji,
    /// variation selectors and tags), plus CJK punctuation (U+3000–303F), kana (U+3040–30FF,
    /// U+31F0–31FF) and full-width forms (U+FF00–FFEF), with every layout feature. The same inputs
    /// give the same bytes. A face with CFF outlines is an [`Error::NotTrueType`].
    ///
    /// The catalogs are also checked as by [`Embed::check_text`]. Relative paths are under the
    /// package directory; an unreadable catalog is a `cargo::warning`.
    #[cfg(feature = "subset")]
    pub fn subset_to<P: Into<PathBuf>>(mut self, catalogs: impl IntoIterator<Item = P>) -> Self {
        self.ui = true;
        self.catalogs.get_or_insert_default().extend(catalogs.into_iter().map(Into::into));
        self
    }

    /// Check that the embedded faces cover these text files, such as translation catalogs: one
    /// `cargo::warning` per file that has characters from U+2E80 up (CJK and later blocks, without
    /// emoji and variation selectors) that no embedded face maps. Those characters fall back to
    /// system fonts at run time. Relative paths are under the package directory. The check runs
    /// only when a checkout was read.
    pub fn check_text<P: Into<PathBuf>>(mut self, paths: impl IntoIterator<Item = P>) -> Self {
        self.texts.extend(paths.into_iter().map(Into::into));
        self
    }

    /// Families the app embeds in another table, such as its interface faces next to its document
    /// fonts: [`Embed::check_text`] and the catalog check count their characters as covered. Each
    /// is read from the checkout whole, so for a face cut elsewhere this is exact only for the
    /// catalogs it was cut to. A name the manifest lacks is a `cargo::warning`.
    pub fn embedded_elsewhere(mut self, families: &[&str]) -> Self {
        self.elsewhere.extend(families.iter().map(|f| (*f).to_owned()));
        self
    }

    /// Read this checkout instead of `CRAFT_FONTS_DIR`.
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Override `CRAFT_FONTS_REQUIRED` (required only when it is exactly `1`).
    pub fn required(mut self, required: bool) -> Self {
        self.required = Some(required);
        self
    }

    /// Copy the selected faces into `dir` instead of embedding them, for a web build that serves
    /// them beside its wasm, and write a table of [`ServedFace`](crate::ServedFace)s.
    ///
    /// Each font file and its licence file keep their path under the checkout's `fonts/`
    /// directory (`noto-sans-arabic/NotoSansArabic.ttf`), and a face's `url` is `url_prefix`
    /// followed by that path, so `serve("web/fonts", "fonts/")` suits a page that serves
    /// `web/fonts` as `fonts/`. An interface face cut by `Embed::subset_to` is served from its cut,
    /// with the start of the cut's SHA-256 in the file name (`ui/NotoSansTCUI-Regular.<hash>.ttf`)
    /// and the cut's own integrity value. A relative `dir` is under the package directory, where
    /// Cargo runs build scripts. Files already there with the right size are left as they are;
    /// files of faces no longer selected, and earlier cuts, are not removed.
    pub fn serve(mut self, dir: impl Into<PathBuf>, url_prefix: &str) -> Self {
        self.serve = Some(Serve { dir: dir.into(), url_prefix: url_prefix.to_owned() });
        self
    }

    /// Write the face table to `out_file` and print the `cargo::` lines; returns the faces
    /// embedded.
    ///
    /// Without a checkout the table is empty. A checkout that can't be read is a
    /// `cargo::warning` and an empty table, or an [`Error`] when required. The table is written
    /// in every case, so on an error the build fails only on the message the caller prints.
    pub fn write(&self, out_file: impl AsRef<Path>) -> Result<Vec<ManifestEntry>, Error> {
        let env = Env { dir: std::env::var_os(DIR_VAR), required: std::env::var_os(REQUIRED_VAR), out_dir: std::env::var_os("OUT_DIR") };
        let mut cargo = Vec::new();
        let result = self.run(&env, out_file.as_ref(), &mut cargo);
        for line in cargo {
            println!("{line}");
        }
        result
    }

    pub(crate) fn run(&self, env: &Env, out_file: &Path, cargo: &mut Vec<String>) -> Result<Vec<ManifestEntry>, Error> {
        cargo.push(format!("cargo::rerun-if-env-changed={DIR_VAR}"));
        cargo.push(format!("cargo::rerun-if-env-changed={REQUIRED_VAR}"));
        for text in self.checked_files() {
            cargo.push(format!("cargo::rerun-if-changed={}", text.display()));
        }
        let out = match (out_file.is_absolute(), &env.out_dir) {
            (true, _) => out_file.to_path_buf(),
            (false, Some(out_dir)) => Path::new(out_dir).join(out_file),
            (false, None) => return Err(Error::NoOutDir),
        };
        let required = self.required.unwrap_or_else(|| env.required.as_ref().is_some_and(|v| v == "1"));
        let dir = self.dir.clone().or_else(|| env.dir.clone().filter(|d| !d.is_empty()).map(PathBuf::from));

        let selected = match &dir {
            None => Err(Error::NotSet),
            Some(dir) => self.select(dir, cargo).and_then(|faces| if required { self.check_required(dir, faces) } else { Ok(faces) }),
        };
        let mut selected = selected;
        if let Ok(faces) = &mut selected {
            // A face that can't be cut is a broken manifest line: an error even when not required.
            if let Err(e) = self.cut(faces, out.parent().unwrap_or(Path::new(""))) {
                write(&out, EMPTY_TABLE)?;
                return Err(e);
            }
            if let Some(dir) = &dir {
                self.check_coverage(faces, dir, cargo);
            }
        }
        let selected = match (selected, &self.serve, &dir) {
            (Ok(faces), Some(serve), Some(dir)) => match serve.copy(dir, &faces, cargo) {
                Ok(table) => Ok((table, faces)),
                Err(e) => {
                    write(&out, EMPTY_TABLE)?;
                    return Err(e);
                }
            },
            (selected, ..) => selected.and_then(|faces| Ok((table(&faces)?, faces))),
        };
        let (table, faces) = match selected {
            Ok((table, faces)) => (table, faces.into_iter().map(|face| face.entry).collect()),
            Err(e) if required => {
                write(&out, EMPTY_TABLE)?;
                return Err(e);
            }
            Err(Error::NotSet) => (EMPTY_TABLE.to_owned(), Vec::new()),
            Err(e) => {
                cargo.push(format!("cargo::warning=building without craft-fonts: {e}"));
                (EMPTY_TABLE.to_owned(), Vec::new())
            }
        };
        write(&out, &table)?;
        Ok(faces)
    }

    /// The wanted manifest entries with their canonical file paths.
    fn select(&self, dir: &Path, cargo: &mut Vec<String>) -> Result<Vec<Face>, Error> {
        let manifest = dir.join("fonts/manifest.txt");
        let text = std::fs::read_to_string(&manifest).map_err(|source| Error::Read { path: manifest.clone(), source })?;
        // Cargo treats a missing watched file as changed, which would rerun the build every time.
        cargo.push(format!("cargo::rerun-if-changed={}", manifest.display()));
        let entries = parse_manifest(&text).map_err(|error| Error::Parse { manifest: manifest.clone(), error })?;
        if let Some(families) = &self.families {
            let unknown: Vec<String> = families.iter().filter(|f| !entries.iter().any(|e| &e.family == *f)).cloned().collect();
            if !unknown.is_empty() {
                let mut known: Vec<String> = Vec::new();
                for entry in &entries {
                    if !known.contains(&entry.family) {
                        known.push(entry.family.clone());
                    }
                }
                return Err(Error::UnknownFamilies { manifest, families: unknown, known });
            }
        }
        let has_ui = entries.iter().any(ManifestEntry::is_ui);
        let no_catalogs = self.catalogs.as_ref().is_none_or(Vec::is_empty);
        let mut faces = Vec::new();
        let mut uncut = Vec::new();
        let mut dropped_cjk = false;
        for entry in entries {
            // Interface faces are the sources of the cuts: embedded whole only when named.
            let named = match &self.families {
                Some(families) => families.contains(&entry.family),
                None => self.ui || !entry.is_ui(),
            };
            if !named {
                continue;
            }
            let wanted = self.scripts.as_ref().is_none_or(|tags| tags.iter().any(|t| entry.has_script(t)));
            let ui_dropped = self.ui && entry.is_cjk() && !entry.is_ui();
            if !wanted || ui_dropped || self.filter.as_ref().is_some_and(|keep| !keep(&entry)) {
                dropped_cjk |= wanted && ui_dropped;
                continue;
            }
            let cut = self.ui && entry.is_ui();
            // An empty list would cut the faces to kana and punctuation, so it counts as none.
            if cut && no_catalogs {
                uncut.push(format!("{} {}", entry.family, entry.style));
                continue;
            }
            let file = dir.join(&entry.file);
            // `include_bytes!` resolves relative paths from the including source file.
            let path = file.canonicalize().map_err(|source| Error::Read { path: file, source })?;
            cargo.push(format!("cargo::rerun-if-changed={}", path.display()));
            let (index, sha256) = (entry.index, entry.sha256.clone());
            faces.push(Face { entry, path, index, cut, sha256 });
        }
        if !uncut.is_empty() {
            let why = if self.catalogs.is_some() { "Embed::subset_to was given no translation catalogs" } else { "no translation catalogs" };
            cargo.push(format!(
                "cargo::warning=craft-fonts: not embedding the interface faces {}: {why}; pass the catalogs to \
                 Embed::subset_to (feature `subset`) to embed them cut to the catalogs' characters",
                uncut.join(", ")
            ));
        }
        if dropped_cjk && !has_ui {
            cargo.push(format!(
                "cargo::warning=craft-fonts: not embedding any CJK face for the interface: {} has no interface face \
                 (role `ui`); use a craft-fonts checkout that has them",
                manifest.display()
            ));
        }
        Ok(faces)
    }

    /// Replace each interface face with its cut, written to `out_dir`.
    #[cfg(feature = "subset")]
    fn cut(&self, faces: &mut [Face], out_dir: &Path) -> Result<(), Error> {
        let Some(catalogs) = &self.catalogs else { return Ok(()) };
        // `select` leaves the faces to cut out when there are no catalogs.
        if !faces.iter().any(|face| face.cut) {
            return Ok(());
        }
        let mut chars = BTreeSet::new();
        // An unreadable catalog adds nothing here; `check_coverage` reports it.
        for text in catalogs.iter().filter_map(|path| std::fs::read_to_string(path).ok()) {
            chars.extend(text.chars().filter(|&c| needs_glyph(c)));
        }
        for (start, end) in UI_RANGES {
            chars.extend(start..=end);
        }
        let dir = out_dir.join("craft-fonts-ui");
        std::fs::create_dir_all(&dir).map_err(|source| Error::Write { path: dir.clone(), source })?;
        for face in faces.iter_mut().filter(|face| face.cut) {
            let source = std::fs::read(&face.path).map_err(|source| Error::Read { path: face.path.clone(), source })?;
            let bytes = crate::subset::subset(&source, face.index, &chars).map_err(|failure| match failure {
                crate::subset::Failure::NotTrueType => Error::NotTrueType { path: face.path.clone() },
                crate::subset::Failure::Other(reason) => Error::Subset { path: face.path.clone(), reason },
            })?;
            let name: String =
                format!("{}-{}", face.entry.family, face.entry.style).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
            let out = dir.join(format!("{name}.ttf"));
            // Rewriting identical bytes would make Cargo rebuild the crate that embeds them.
            if std::fs::read(&out).ok().as_deref() != Some(&bytes[..]) {
                std::fs::write(&out, &bytes).map_err(|source| Error::Write { path: out.clone(), source })?;
            }
            face.path = out;
            face.index = 0;
            face.sha256 = crate::sha256::hex(&bytes);
        }
        Ok(())
    }

    /// Without the subsetter there are no catalogs, so [`Embed::select`] left out every face to cut.
    #[cfg(not(feature = "subset"))]
    fn cut(&self, _faces: &mut [Face], _out_dir: &Path) -> Result<(), Error> {
        Ok(())
    }

    /// The catalogs, then the other [`Embed::check_text`] files.
    fn checked_files(&self) -> Vec<&PathBuf> {
        let mut files: Vec<&PathBuf> = self.catalogs.iter().flatten().collect();
        for text in &self.texts {
            if !files.contains(&text) {
                files.push(text);
            }
        }
        files
    }

    /// The manifest entries of the [`Embed::embedded_elsewhere`] families, with a
    /// `cargo::warning` for a family the manifest lacks.
    fn elsewhere(&self, dir: &Path, cargo: &mut Vec<String>) -> Vec<ManifestEntry> {
        if self.elsewhere.is_empty() {
            return Vec::new();
        }
        let manifest = dir.join("fonts/manifest.txt");
        // `select` read and parsed this manifest already.
        let entries = std::fs::read_to_string(&manifest).ok().and_then(|text| parse_manifest(&text).ok()).unwrap_or_default();
        for family in self.elsewhere.iter().filter(|f| !entries.iter().any(|e| &e.family == *f)) {
            cargo.push(format!("cargo::warning=craft-fonts: embedded_elsewhere names {family}, which {} lacks", manifest.display()));
        }
        entries.into_iter().filter(|e| self.elsewhere.contains(&e.family)).collect()
    }

    /// Warn about each catalog's and [`Embed::check_text`] file's characters that neither an
    /// embedded face nor an [`Embed::embedded_elsewhere`] face covers.
    fn check_coverage(&self, faces: &[Face], dir: &Path, cargo: &mut Vec<String>) {
        let files = self.checked_files();
        if files.is_empty() {
            return;
        }
        let elsewhere = self.elsewhere(dir, cargo);
        let fonts: Vec<(&ManifestEntry, u32, Option<Vec<u8>>)> = faces
            .iter()
            .map(|face| (&face.entry, face.index, std::fs::read(&face.path).ok()))
            .chain(elsewhere.iter().map(|entry| (entry, entry.index, std::fs::read(dir.join(&entry.file)).ok())))
            .collect();
        let mut cmaps = Vec::new();
        for (entry, index, bytes) in &fonts {
            match bytes.as_deref().and_then(|bytes| Cmap::read(bytes, *index)) {
                Some(cmap) => cmaps.push(cmap),
                None => cargo
                    .push(format!("cargo::warning=craft-fonts: cannot read the character map of {} {} ({})", entry.family, entry.style, entry.file)),
            }
        }
        for path in files {
            let text = match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(e) => {
                    cargo.push(format!("cargo::warning=craft-fonts: cannot check {}: {e}", path.display()));
                    continue;
                }
            };
            let mut seen = HashSet::new();
            let missing: Vec<char> = text.chars().filter(|&c| needs_glyph(c) && seen.insert(c) && !cmaps.iter().any(|cmap| cmap.covers(c))).collect();
            if missing.is_empty() {
                continue;
            }
            const SHOWN: usize = 8;
            let shown: String = missing.iter().take(SHOWN).collect();
            let more = if missing.len() > SHOWN { " …" } else { "" };
            cargo.push(format!(
                "cargo::warning=craft-fonts: {} has {} character{} no embedded face covers ({shown}{more}); system fonts will draw them.",
                path.display(),
                missing.len(),
                if missing.len() == 1 { "" } else { "s" },
            ));
        }
    }

    fn check_required(&self, dir: &Path, faces: Vec<Face>) -> Result<Vec<Face>, Error> {
        let manifest = dir.join("fonts/manifest.txt");
        if faces.is_empty() {
            return Err(Error::NoFaces { manifest });
        }
        let missing: Vec<String> = self
            .scripts
            .iter()
            .flatten()
            .filter(|tag| !faces.iter().any(|face| face.entry.scripts.iter().any(|s| script_matches(s, tag))))
            .cloned()
            .collect();
        if missing.is_empty() { Ok(faces) } else { Err(Error::MissingScripts { manifest, scripts: missing }) }
    }
}

/// Whether the interface faces need `c`: CJK and later blocks, except emoji and other pictographs
/// (the emoji faces draw them), variation selectors, tags and a byte order mark.
fn needs_glyph(c: char) -> bool {
    c >= '\u{2E80}' && !matches!(c, '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{1F000}'..='\u{1FFFF}' | '\u{E0000}'..)
}

/// Kept in every cut, whatever the catalogs use: CJK punctuation, kana and full-width forms.
#[cfg(feature = "subset")]
const UI_RANGES: [(char, char); 4] = [('\u{3000}', '\u{303F}'), ('\u{3040}', '\u{30FF}'), ('\u{31F0}', '\u{31FF}'), ('\u{FF00}', '\u{FFEF}')];

/// The environment a build script sees, separated out for tests.
pub(crate) struct Env {
    pub dir: Option<OsString>,
    pub required: Option<OsString>,
    pub out_dir: Option<OsString>,
}

/// The generated source; `include_bytes!` takes a string, so a face under a path that isn't UTF-8
/// is an error rather than a lossy path the consumer's build can't find.
fn table(faces: &[Face]) -> Result<String, Error> {
    let mut src = String::from("// Generated by craft-fonts-build. Do not edit.\n&[\n");
    for Face { entry, path, index, .. } in faces {
        let file = path.to_str().ok_or_else(|| Error::NonUtf8Path { path: path.clone() })?;
        let scripts: Vec<String> = entry.scripts.iter().map(|s| format!("{s:?}")).collect();
        let _ = writeln!(
            src,
            "    EmbeddedFace {{ family: {:?}, style: {:?}, scripts: &[{}], index: {}, bytes: include_bytes!({:?}) }},",
            entry.family,
            entry.style,
            scripts.join(", "),
            index,
            file,
        );
    }
    src.push_str("]\n");
    Ok(src)
}

impl Serve {
    /// Copy the faces' files and licences into the served directory; returns the table.
    fn copy(&self, checkout: &Path, faces: &[Face], cargo: &mut Vec<String>) -> Result<String, Error> {
        let mut copied: Vec<String> = Vec::new();
        let mut src = String::from("// Generated by craft-fonts-build. Do not edit.\n&[\n");
        for face in faces {
            let (entry, path) = (&face.entry, &face.path);
            let served = served_file(face);
            let font = std::fs::read(path).map_err(|source| Error::Read { path: path.clone(), source })?;
            let size = font.len();
            if !copied.contains(&served) {
                copied.push(served.clone());
                copy_file(path, &self.dir.join(&served), cargo)?;
                let licence = checkout.join(&entry.licence_file);
                if entry.licence_file.is_empty() || !licence.is_file() {
                    cargo.push(format!("cargo::warning=craft-fonts: no licence file beside {}", entry.file));
                } else {
                    copy_file(&licence, &self.dir.join(served_path(&entry.licence_file)), cargo)?;
                }
            }
            let coverage = crate::coverage::of_face(&font, face.index).unwrap_or_else(|| {
                cargo.push(format!("cargo::warning=craft-fonts: no character map in {}; the face is never fetched for a character", entry.file));
                Vec::new()
            });
            let scripts: Vec<String> = entry.scripts.iter().map(|s| format!("{s:?}")).collect();
            let _ = writeln!(
                src,
                "    ServedFace {{ family: {:?}, style: {:?}, scripts: &[{}], index: {}, url: {:?}, size: {size}, integrity: {:?}, coverage: {} }},",
                entry.family,
                entry.style,
                scripts.join(", "),
                face.index,
                format!("{}{served}", self.url_prefix),
                integrity(&face.sha256),
                byte_string(&coverage),
            );
        }
        src.push_str("]\n");
        Ok(src)
    }
}

/// A byte string literal, printable ASCII as itself.
fn byte_string(bytes: &[u8]) -> String {
    let mut out = String::from("b\"");
    for &b in bytes {
        match b {
            b'"' | b'\\' => {
                out.push('\\');
                out.push(char::from(b));
            }
            0x20..=0x7E => out.push(char::from(b)),
            _ => {
                let _ = write!(out, "\\x{b:02x}");
            }
        }
    }
    out.push('"');
    out
}

/// Where a face's file is served, under the served directory: its checkout path under `fonts/`.
/// A cut adds the start of its SHA-256 before the extension (`ui/A.1234abcd5678ef90.ttf`), so a
/// cut from changed catalogs gets a new URL instead of meeting a cached copy that fails its
/// integrity check.
fn served_file(face: &Face) -> String {
    let file = served_path(&face.entry.file);
    if !face.cut {
        return file.to_owned();
    }
    let hash = face.sha256.get(..16).unwrap_or(&face.sha256);
    match file.rsplit_once('.') {
        Some((stem, extension)) if !extension.contains('/') => format!("{stem}.{hash}.{extension}"),
        _ => format!("{file}.{hash}"),
    }
}

/// A checkout path relative to the served directory: `fonts/a/A.ttf` → `a/A.ttf`.
fn served_path(file: &str) -> &str {
    file.strip_prefix("fonts/").unwrap_or(file)
}

/// Copy `from` unless `to` already has its size. The copy keeps `from`'s modification time and is
/// watched, so the build copies it again once it is deleted, without running every build.
fn copy_file(from: &Path, to: &Path, cargo: &mut Vec<String>) -> Result<(), Error> {
    cargo.push(format!("cargo::rerun-if-changed={}", to.display()));
    let write_error = |source| Error::Write { path: to.to_path_buf(), source };
    let size = |path: &Path| std::fs::metadata(path).ok().map(|m| m.len());
    if size(to).is_some() && size(to) == size(from) {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(write_error)?;
    }
    std::fs::copy(from, to).map_err(write_error)?;
    // Cargo reruns a build script whose watched files are newer than its last run.
    let modified = std::fs::metadata(from).and_then(|m| m.modified()).map_err(|source| Error::Read { path: from.to_path_buf(), source })?;
    std::fs::File::options().write(true).open(to).and_then(|f| f.set_modified(modified)).map_err(write_error)
}

/// The Subresource Integrity value (`sha256-<base64>`) of a hex SHA-256, or `""` for a value that
/// isn't one.
pub(crate) fn integrity(sha256: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let digest: Option<Vec<u8>> =
        (sha256.len() == 64).then(|| (0..32).map(|i| u8::from_str_radix(sha256.get(2 * i..2 * i + 2)?, 16).ok()).collect()).flatten();
    let Some(digest) = digest else { return String::new() };
    let mut out = String::from("sha256-");
    for chunk in digest.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            let c = if i <= chunk.len() { ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] } else { b'=' };
            out.push(char::from(c));
        }
    }
    out
}

fn write(path: &Path, contents: &str) -> Result<(), Error> {
    std::fs::write(path, contents).map_err(|source| Error::Write { path: path.to_path_buf(), source })
}

/// Why the faces could not be embedded. Print it as `cargo::error={error}`.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// `CRAFT_FONTS_REQUIRED=1` but `CRAFT_FONTS_DIR` is unset or empty.
    NotSet,
    /// The manifest or a font file it lists can't be read.
    Read { path: PathBuf, source: io::Error },
    /// The manifest has a malformed line.
    Parse { manifest: PathBuf, error: ParseError },
    /// Required, but no face in the manifest was selected.
    NoFaces { manifest: PathBuf },
    /// Required, but no face in the manifest has these requested script tags.
    MissingScripts { manifest: PathBuf, scripts: Vec<String> },
    /// The generated table can't be written.
    Write { path: PathBuf, source: io::Error },
    /// `out_file` is relative and `OUT_DIR` is unset (not running as a build script).
    NoOutDir,
    /// An interface face has CFF outlines, which can't be cut correctly; it must be TrueType.
    NotTrueType { path: PathBuf },
    /// An interface face can't be cut.
    Subset { path: PathBuf, reason: String },
    /// [`Embed::documents`] names families the manifest lacks; `known` lists the manifest's.
    UnknownFamilies { manifest: PathBuf, families: Vec<String>, known: Vec<String> },
    /// A face to embed has a path that isn't UTF-8, which `include_bytes!` can't name.
    NonUtf8Path { path: PathBuf },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSet => {
                write!(f, "{REQUIRED_VAR}=1 but {DIR_VAR} is not: set {DIR_VAR} to the absolute path of a storytold/craft-fonts checkout")
            }
            Self::Read { path, source } => {
                write!(f, "cannot read {} ({DIR_VAR} must be the absolute path of a storytold/craft-fonts checkout): {source}", path.display())
            }
            Self::Parse { manifest, error } => write!(f, "{}: {error}", manifest.display()),
            Self::NoFaces { manifest } => write!(f, "{REQUIRED_VAR}=1 but no face in {} was selected", manifest.display()),
            Self::MissingScripts { manifest, scripts } => {
                write!(f, "{REQUIRED_VAR}=1 but {} has no face for {}", manifest.display(), scripts.join(", "))
            }
            Self::Write { path, source } => write!(f, "cannot write {}: {source}", path.display()),
            Self::NoOutDir => write!(f, "OUT_DIR is not set: call craft-fonts-build from a build script, or pass an absolute out_file"),
            Self::NotTrueType { path } => write!(
                f,
                "{} has CFF outlines, which craft-fonts-build can't subset correctly: an interface face (role ui) must be a TrueType (glyf) font",
                path.display()
            ),
            Self::Subset { path, reason } => write!(f, "cannot subset {}: {reason}", path.display()),
            Self::UnknownFamilies { manifest, families, known } => write!(
                f,
                "{} has no family {}; Embed::documents names families as the manifest spells them: {}",
                manifest.display(),
                families.join(", "),
                known.join(", ")
            ),
            Self::NonUtf8Path { path } => write!(f, "cannot embed {}: include_bytes! needs a UTF-8 path", path.display()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            Self::Parse { error, .. } => Some(error),
            _ => None,
        }
    }
}
