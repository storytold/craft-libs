//! Font indexes (aiongg/craft-font-index, format 1.x): the families of every source in one
//! [`Index`], and exact lookup.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::source::{Origin, fill, parse};

/// The largest index read.
pub const MAX_INDEX_BYTES: usize = 32 << 20;
/// The largest font file an index may list.
pub const MAX_FILE_BYTES: u64 = 256 << 20;
/// The largest licence file an index may list.
pub const MAX_LICENSE_BYTES: u64 = 1 << 20;
/// The index format's major version this crate reads.
pub const MAJOR_VERSION: u32 = 1;
/// The source ID of Google Fonts, and of every format 1.0 index.
pub const GOOGLE_FONTS: &str = "google/fonts";
/// The licences that allow redistribution. Families under any other are left out.
pub const LICENSES: [&str; 3] = ["OFL", "APACHE2", "UFL"];

/// Why an index can't be used.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// More than [`MAX_INDEX_BYTES`].
    #[error("the font index is larger than {max} bytes")]
    TooLarge {
        /// The limit.
        max: usize,
    },
    /// Not JSON, or a field has the wrong type.
    #[error("the font index can't be read: {0}")]
    Json(String),
    /// A major version this crate doesn't know.
    #[error("the font index has version {0}; this build reads version {MAJOR_VERSION}.x")]
    Version(String),
    /// An entry breaks the format's rules.
    #[error("the font index has an invalid entry: {0}")]
    Invalid(String),
    /// A gzipped index that doesn't decompress.
    #[error("the gzipped font index can't be read: {0}")]
    Gzip(String),
    /// The bytes aren't the pinned ones.
    #[error("the font index doesn't have the sha256 this build expects")]
    Checksum,
}

/// A language some family supports.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Language {
    /// gflanguages ID: `<ISO 639>_<ISO 15924>`, sometimes with `_<region>` (`vi_Latn`).
    pub id: String,
    /// English name.
    pub name: String,
    /// The name in the language itself.
    #[serde(default)]
    pub autonym: Option<String>,
    /// ISO 15924 code.
    #[serde(default)]
    pub script: String,
    /// Speakers; 0 when unknown.
    #[serde(default)]
    pub population: u64,
    /// A historical language.
    #[serde(default)]
    pub historical: bool,
}

/// A script some family uses.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Script {
    /// ISO 15924 code (`Zmth`, `Zsym` and `Zsye` for maths, symbols and emoji).
    pub id: String,
    /// English name.
    pub name: String,
}

/// A variation axis of a font file.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Axis {
    /// The axis tag (`wght`).
    pub tag: String,
    /// The smallest value.
    pub min: f32,
    /// The largest value.
    pub max: f32,
}

/// One font file of a family.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FontFile {
    /// Path in the family's repository: the `{path}` of the source's URL templates.
    pub path: String,
    /// Bytes.
    pub size: u64,
    /// 64 lowercase hex characters.
    pub sha256: String,
    /// `normal` or `italic`.
    #[serde(default = "normal")]
    pub style: String,
    /// CSS weight; for a variable font, the default instance's.
    pub weight: u16,
    /// CSS font-stretch percentage; for a variable font, the default instance's.
    #[serde(default = "normal_width")]
    pub width: f32,
    /// Variation axes; empty for a static font.
    #[serde(default)]
    pub axes: Vec<Axis>,
    /// PostScript names: the face's and, for a variable font, each named instance's.
    #[serde(default)]
    pub postscript: Vec<String>,
    /// The repository holding the file, when not the family's.
    #[serde(default)]
    pub repo: Option<String>,
    /// The commit or tag of `repo`, when not the family's.
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
}

fn normal() -> String {
    "normal".into()
}

fn normal_width() -> f32 {
    100.0
}

impl FontFile {
    /// The file's name, the last part of its path.
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// Whether the file's faces are italic.
    pub fn is_italic(&self) -> bool {
        self.style.eq_ignore_ascii_case("italic")
    }

    /// Whether it is a variable font.
    pub fn is_variable(&self) -> bool {
        !self.axes.is_empty()
    }

    fn axis(&self, tag: &str) -> Option<&Axis> {
        self.axes.iter().find(|a| a.tag == tag)
    }

    /// Whether the file has the face of CSS `weight`, italic or not: its own weight, or a weight
    /// its `wght` axis reaches.
    pub fn has_weight(&self, weight: u16, italic: bool) -> bool {
        let italic_ok = self.is_italic() == italic || self.axis("ital").is_some_and(|a| a.min <= 0.0 && a.max >= 1.0);
        let w = f32::from(weight);
        italic_ok && (self.weight == weight || self.axis("wght").is_some_and(|a| a.min <= w && w <= a.max))
    }

    /// Whether the file has the normal width (100%), as its own or on its `wdth` axis.
    pub fn has_normal_width(&self) -> bool {
        self.width == 100.0 || self.axis("wdth").is_some_and(|a| a.min <= 100.0 && 100.0 <= a.max)
    }
}

/// A family's licence file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LicenseFile {
    /// Path in its repository.
    pub path: String,
    /// Bytes.
    pub size: u64,
    /// 64 lowercase hex characters.
    pub sha256: String,
    /// The repository holding it, when not the family's.
    #[serde(default)]
    pub repo: Option<String>,
    /// The commit or tag of `repo`, when not the family's.
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
}

impl LicenseFile {
    /// The file's name (`OFL.txt`, `LICENSE`).
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// A font family of an index.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Family {
    /// The family name, unique in its source ignoring case. Two sources can have the same name.
    pub family: String,
    /// The ID of the family's source ([`FontSource::id`]).
    #[serde(default)]
    pub source: String,
    /// The designer; empty when not given.
    #[serde(default)]
    pub designer: String,
    /// `SANS_SERIF`, `SERIF`, `DISPLAY`, `HANDWRITING` or `MONOSPACE`.
    #[serde(default)]
    pub categories: Vec<String>,
    /// google/fonts subsets (`latin`, `japanese`, ...); empty for other sources.
    #[serde(default)]
    pub subsets: Vec<String>,
    /// Positions in [`Index::languages`].
    #[serde(default)]
    pub languages: Vec<usize>,
    /// ISO 15924 codes.
    #[serde(default)]
    pub scripts: Vec<String>,
    /// `OFL`, `APACHE2` or `UFL`.
    #[serde(default)]
    pub license: String,
    /// Format 1.0: the licence file in google/fonts, beside the font files.
    #[serde(default)]
    pub license_path: Option<String>,
    /// The licence file, with its size and sha256.
    #[serde(default)]
    pub license_file: Option<LicenseFile>,
    /// The repository holding the files, when not the source's.
    #[serde(default)]
    pub repo: Option<String>,
    /// The commit or tag of `repo`, when not the source's.
    #[serde(default, rename = "ref")]
    pub git_ref: Option<String>,
    /// The font files, sorted by path.
    pub files: Vec<FontFile>,
    #[serde(skip)]
    pub(crate) dir: String,
    #[serde(skip)]
    pub(crate) curated: bool,
}

impl Family {
    /// The family's folder in the font store: google/fonts' folder name (`notosansjp`), or
    /// `<source>/<family>` (`notofonts/noto-sans-cjk-jp`) for other sources.
    pub fn dir(&self) -> &str {
        &self.dir
    }

    /// Whether the family's source is curated: a built-in source, not one the user added.
    pub fn is_curated(&self) -> bool {
        self.curated
    }

    /// The licence file's name (`OFL.txt`, `LICENSE`).
    pub fn license_file_name(&self) -> Option<&str> {
        match &self.license_file {
            Some(f) => Some(f.file_name()),
            None => self.license_path.as_deref().and_then(|p| p.rsplit('/').next()),
        }
    }

    /// The file with exactly this style: a style name (`Bold Italic`, `SemiBold`, `Regular`), a
    /// CSS weight (`700`), or the part of a PostScript name after its `-` (`BoldItalic`,
    /// `CondensedBold`). A variable font has every weight its `wght` axis reaches. A static file
    /// of normal width is preferred. No nearer style is offered: a style the family doesn't have
    /// is [`Unavailable::NoStyle`].
    pub fn face(&self, style: &str) -> Result<&FontFile, Unavailable> {
        let no_style = || Unavailable::NoStyle { family: self.family.clone(), style: style.to_string() };
        if let Some((weight, italic)) = parse_style(style) {
            let found: Vec<&FontFile> = self.files.iter().filter(|f| f.has_weight(weight, italic) && f.has_normal_width()).collect();
            let exact = found.iter().find(|f| !f.is_variable() && f.weight == weight && f.is_italic() == italic && f.width == 100.0);
            if let Some(f) = exact.or(found.first()) {
                return Ok(f);
            }
        }
        let wanted = squash(style);
        if wanted.is_empty() {
            return Err(no_style());
        }
        self.files.iter().find(|f| f.postscript.iter().any(|p| p.split_once('-').is_some_and(|(_, s)| squash(s) == wanted))).ok_or_else(no_style)
    }

    /// The file with this PostScript name, matched exactly.
    pub fn face_by_postscript(&self, name: &str) -> Option<&FontFile> {
        self.files.iter().find(|f| f.postscript.iter().any(|p| p == name))
    }

    /// The family's languages.
    pub fn languages<'a>(&'a self, index: &'a Index) -> impl Iterator<Item = &'a Language> + 'a {
        self.languages.iter().filter_map(|&i| index.languages.get(i))
    }

    /// The total size of the family's files.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Letters and digits only, lowercased: `Bold Italic`, `bold-italic` and `BoldItalic` are equal.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A style name's CSS weight and whether it is italic: `Bold Italic` is (700, true), `Regular`
/// and the empty style are (400, false), `300` is (300, false). `None` for names that aren't a
/// weight (`Condensed Bold`).
pub fn parse_style(style: &str) -> Option<(u16, bool)> {
    let s = squash(style);
    let (base, italic) = match s.strip_suffix("italic").or_else(|| s.strip_suffix("oblique")) {
        Some(b) => (b, true),
        None => (s.as_str(), false),
    };
    let weight = match base {
        "" | "regular" | "normal" | "book" | "roman" => 400,
        "thin" | "hairline" => 100,
        "extralight" | "ultralight" => 200,
        "light" => 300,
        "medium" => 500,
        "semibold" | "demibold" => 600,
        "bold" => 700,
        "extrabold" | "ultrabold" => 800,
        "black" | "heavy" => 900,
        n => n.parse::<u16>().ok().filter(|w| (1..=1000).contains(w))?,
    };
    Some((weight, italic))
}

/// Why the index has no exact match. Near matches are never offered as installable.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unavailable {
    /// No family of this name.
    #[error("{0} is not in the font index")]
    NoFamily(String),
    /// The family exists, without this style.
    #[error("the font index has {family} but not its {style} style")]
    NoStyle {
        /// The family's name in the index.
        family: String,
        /// The style asked for.
        style: String,
    },
    /// No file has this PostScript name.
    #[error("no font in the index has the PostScript name {0}")]
    NoPostScript(String),
    /// Files of several families have this PostScript name.
    #[error("the PostScript name {name} belongs to several families: {}", families.join(", "))]
    Ambiguous {
        /// The PostScript name.
        name: String,
        /// The families that have it.
        families: Vec<String>,
    },
}

/// An exact match: a family and the file that has the face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face<'a> {
    /// The family.
    pub family: &'a Family,
    /// The file.
    pub file: &'a FontFile,
}

/// A source of fonts: one index file.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSource {
    /// The ID its families carry: `google/fonts`, `notofonts`, or `user:<host>/<id>` for a
    /// source the user added.
    pub id: String,
    /// Display name (`Google Fonts`, `Noto`).
    pub name: String,
    /// The source's web page.
    pub homepage: Option<String>,
    /// A built-in source. A source the user added is never curated, whatever its file says.
    pub curated: bool,
    /// The index format version, `"MAJOR.MINOR"`.
    pub version: String,
    /// The commit the index was generated from.
    pub commit: String,
    /// The URL a user-added source was added from.
    pub user_url: Option<String>,
    /// A user-added source whose file differs from the one first fetched.
    pub changed: bool,
    /// Families left out of the index and why (a licence that doesn't allow redistribution).
    pub warnings: Vec<String>,
    /// How many families it has.
    pub families: usize,
    pub(crate) templates: Vec<String>,
    pub(crate) repo: Option<String>,
    pub(crate) git_ref: Option<String>,
}

impl FontSource {
    /// The URL templates of its files, best first.
    pub fn file_url_templates(&self) -> &[String] {
        &self.templates
    }
}

/// A source that couldn't be loaded. The other sources still are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProblem {
    /// The source's ID, or its URL for a user-added source.
    pub source: String,
    /// Its display name, or its URL.
    pub name: String,
    /// What went wrong.
    pub error: String,
}

pub(crate) type Part = (FontSource, Vec<Language>, Vec<Script>, Vec<Family>);

/// The checked indexes of one or more sources. Families are sorted by name; families of the same
/// name are in source order (built-in sources in `sources.json` order, then the user's), and
/// lookups by name prefer the first.
#[derive(Clone, Debug, Default)]
pub struct Index {
    sources: Vec<FontSource>,
    problems: Vec<SourceProblem>,
    languages: Vec<Language>,
    scripts: Vec<Script>,
    families: Vec<Family>,
    by_name: HashMap<String, Vec<usize>>,
    by_postscript: HashMap<String, Vec<(usize, usize)>>,
}

impl Index {
    /// Reads one source's index from its JSON, checking every entry. Its `curated` flag is
    /// kept; a format 1.0 index is Google Fonts and curated.
    pub fn from_json(bytes: &[u8]) -> Result<Self, IndexError> {
        parse(bytes, Origin::Trusted)
    }

    /// Reads an index from gzipped JSON.
    pub fn from_gzip(bytes: &[u8]) -> Result<Self, IndexError> {
        Self::from_json(&gunzip(bytes, MAX_INDEX_BYTES)?)
    }

    /// Reads an index from JSON or gzipped JSON.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IndexError> {
        if is_gzip(bytes) { Self::from_gzip(bytes) } else { Self::from_json(bytes) }
    }

    /// Reads an index the user added from `url` (JSON or gzipped JSON): not curated, its source
    /// ID `user:<host>/<id>`, so it can't stand in for a built-in source.
    pub fn from_user_bytes(bytes: &[u8], url: &str) -> Result<Self, IndexError> {
        let json = if is_gzip(bytes) { gunzip(bytes, MAX_INDEX_BYTES)? } else { bytes.to_vec() };
        parse(&json, Origin::User { url })
    }

    /// The built-in sources compiled into this build ([`crate::PINNED_INDEX`]): `sources.json`
    /// and every index it lists, each checked against its sha256.
    #[cfg(feature = "bundled-index")]
    pub fn bundled() -> Result<Self, IndexError> {
        crate::sources::bundled()
    }

    /// One index of every source in `parts`, in order. A source whose ID came earlier is left
    /// out and reported in [`Index::problems`].
    pub fn merge(parts: impl IntoIterator<Item = Index>) -> Self {
        let mut list = Vec::new();
        let mut problems = Vec::new();
        for part in parts {
            problems.extend(part.problems);
            let mut families = part.families;
            for source in part.sources {
                let (mine, rest): (Vec<Family>, Vec<Family>) = families.into_iter().partition(|f| f.source == source.id);
                families = rest;
                list.push((source, part.languages.clone(), part.scripts.clone(), mine));
            }
        }
        Self::assemble(list, problems)
    }

    /// The index with `problems` added, for sources that couldn't be loaded.
    pub fn with_problems(mut self, problems: impl IntoIterator<Item = SourceProblem>) -> Self {
        self.problems.extend(problems);
        self
    }

    /// Marks every source as changed since the user added it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn mark_changed(&mut self) {
        for s in &mut self.sources {
            s.changed = true;
        }
    }

    pub(crate) fn assemble(parts: Vec<Part>, mut problems: Vec<SourceProblem>) -> Self {
        let mut sources: Vec<FontSource> = Vec::new();
        let mut languages: BTreeMap<String, Language> = BTreeMap::new();
        let mut scripts: BTreeMap<String, Script> = BTreeMap::new();
        let mut families: Vec<(Family, Vec<String>)> = Vec::new();
        for (source, langs, scrs, fams) in parts {
            if sources.iter().any(|s| s.id == source.id) {
                problems.push(SourceProblem {
                    source: source.user_url.clone().unwrap_or_else(|| source.id.clone()),
                    name: source.name.clone(),
                    error: format!("another source already has the ID {}", source.id),
                });
                continue;
            }
            for f in fams {
                let ids = f.languages.iter().filter_map(|&i| langs.get(i)).map(|l| l.id.clone()).collect();
                families.push((f, ids));
            }
            for l in langs {
                languages.entry(l.id.clone()).or_insert(l);
            }
            for s in scrs {
                scripts.entry(s.id.clone()).or_insert(s);
            }
            sources.push(source);
        }
        let position: HashMap<&str, usize> = languages.keys().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
        let mut merged: Vec<Family> = families
            .into_iter()
            .map(|(mut f, ids)| {
                f.languages = ids.iter().filter_map(|id| position.get(id.as_str()).copied()).collect();
                f
            })
            .collect();
        let order: HashMap<&str, usize> = sources.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        merged.sort_by_cached_key(|f| (f.family.to_lowercase(), order.get(f.source.as_str()).copied().unwrap_or(usize::MAX)));
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_postscript: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (fi, fam) in merged.iter().enumerate() {
            by_name.entry(fam.family.to_lowercase()).or_default().push(fi);
            for (xi, file) in fam.files.iter().enumerate() {
                for ps in &file.postscript {
                    let hits = by_postscript.entry(ps.clone()).or_default();
                    if !hits.contains(&(fi, xi)) {
                        hits.push((fi, xi));
                    }
                }
            }
        }
        Index {
            sources,
            problems,
            languages: languages.into_values().collect(),
            scripts: scripts.into_values().collect(),
            families: merged,
            by_name,
            by_postscript,
        }
    }

    /// The sources, built-in ones first.
    pub fn sources(&self) -> &[FontSource] {
        &self.sources
    }

    /// The source with this ID.
    pub fn source(&self, id: &str) -> Option<&FontSource> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// The source a family comes from.
    pub fn source_of(&self, family: &Family) -> Option<&FontSource> {
        self.source(&family.source)
    }

    /// The sources that couldn't be loaded, and why.
    pub fn problems(&self) -> &[SourceProblem] {
        &self.problems
    }

    /// Every family, sorted by name, then by source.
    pub fn families(&self) -> &[Family] {
        &self.families
    }

    /// Every language a family supports, sorted by ID.
    pub fn languages(&self) -> &[Language] {
        &self.languages
    }

    /// Every script a family uses, sorted by code.
    pub fn scripts(&self) -> &[Script] {
        &self.scripts
    }

    /// The family of this name, ignoring case, from the first source that has it. Only the whole
    /// name matches.
    pub fn family(&self, name: &str) -> Option<&Family> {
        self.families_named(name).next()
    }

    /// Every family of this name, ignoring case, in source order.
    pub fn families_named(&self, name: &str) -> impl Iterator<Item = &Family> + '_ {
        self.by_name.get(&name.to_lowercase()).into_iter().flatten().filter_map(|&i| self.families.get(i))
    }

    /// The family of this name from one source.
    pub fn family_from(&self, source: &str, name: &str) -> Option<&Family> {
        self.families_named(name).find(|f| f.source == source)
    }

    /// The exact face for a family and style ([`Family::face`]), from the first source whose
    /// family of that name has the style.
    pub fn find_face(&self, family: &str, style: &str) -> Result<Face<'_>, Unavailable> {
        let mut first_error = None;
        for fam in self.families_named(family) {
            match fam.face(style) {
                Ok(file) => return Ok(Face { family: fam, file }),
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        Err(first_error.unwrap_or_else(|| Unavailable::NoFamily(family.to_string())))
    }

    /// The face with this PostScript name, matched exactly (case included), from the first
    /// source that has it. When families of different names have it, it is
    /// [`Unavailable::Ambiguous`]: look it up in the document's family with
    /// [`Family::face_by_postscript`].
    pub fn find_postscript(&self, name: &str) -> Result<Face<'_>, Unavailable> {
        let hits = self.by_postscript.get(name).map(Vec::as_slice).unwrap_or_default();
        let mut names: Vec<&str> = Vec::new();
        for &(fi, _) in hits {
            if let Some(f) = self.families.get(fi)
                && !names.iter().any(|n| n.to_lowercase() == f.family.to_lowercase())
            {
                names.push(&f.family);
            }
        }
        match (names.len(), hits.first()) {
            (1, Some(&(fi, xi))) => {
                let family = self.families.get(fi).ok_or_else(|| Unavailable::NoPostScript(name.into()))?;
                let file = family.files.get(xi).ok_or_else(|| Unavailable::NoPostScript(name.into()))?;
                Ok(Face { family, file })
            }
            (0, _) | (_, None) => Err(Unavailable::NoPostScript(name.into())),
            _ => Err(Unavailable::Ambiguous { name: name.into(), families: names.into_iter().map(str::to_string).collect() }),
        }
    }

    /// The URLs of a font file of `family`, best first: the source's templates filled with the
    /// file's path, repository and ref.
    pub fn file_urls(&self, family: &Family, file: &FontFile) -> Vec<String> {
        self.urls(family, &file.path, file.repo.as_deref(), file.git_ref.as_deref())
    }

    /// The URLs of `family`'s licence file, best first; empty when it has none.
    pub fn license_urls(&self, family: &Family) -> Vec<String> {
        match (&family.license_file, &family.license_path) {
            (Some(f), _) => self.urls(family, &f.path, f.repo.as_deref(), f.git_ref.as_deref()),
            (None, Some(path)) => self.urls(family, path, None, None),
            (None, None) => Vec::new(),
        }
    }

    fn urls(&self, family: &Family, path: &str, repo: Option<&str>, git_ref: Option<&str>) -> Vec<String> {
        let Some(source) = self.source_of(family) else { return Vec::new() };
        let repo = repo.or(family.repo.as_deref()).or(source.repo.as_deref());
        let git_ref = git_ref.or(family.git_ref.as_deref()).or(source.git_ref.as_deref());
        fill(&source.templates, path, repo, git_ref).unwrap_or_default()
    }
}

pub(crate) fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

/// Decompresses at most `max` bytes; more is an error, not a truncated index.
pub(crate) fn gunzip(bytes: &[u8], max: usize) -> Result<Vec<u8>, IndexError> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).take((max as u64).saturating_add(1)).read_to_end(&mut out).map_err(|e| IndexError::Gzip(e.to_string()))?;
    if out.len() > max {
        return Err(IndexError::TooLarge { max });
    }
    Ok(out)
}
