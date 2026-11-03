//! Installed Language packs: a language's spelling dictionary, hyphenation patterns and licence
//! texts, downloaded and unpacked by the app's font store (craft-libs `craft-font-store`, item
//! kind `language-pack`) and read here at runtime.
//!
//! The packs folder ([`set_packs_dir`]) holds one folder per pack, named by its locale (lowercase
//! BCP 47: `nl`, `pt-br`, `sr-latn`), laid out like the pack archive's root:
//!
//! ```text
//! <packs folder>/<locale>/pack.toml        locale, name, licence, class, sources
//!                        /spelling.aff     Hunspell affix file      (optional)
//!                        /spelling.dic     Hunspell word list       (optional)
//!                        /hyphenation.pat  TeX/Liang patterns       (optional)
//!                        /LICENCES/<file>  the upstream licence texts
//! ```
//!
//! Lookup order for a language's spelling: the dictionaries folder, then an installed pack, then
//! the system's Hunspell dictionaries, then (English only) the core word list built into the
//! crate. For hyphenation: an installed pack, then the patterns embedded at build time.
//!
//! Every file is size-capped and read only when it is a regular file (never through a symbolic
//! link); a pack whose `pack.toml` is missing, unreadable or names a path outside the pack is
//! skipped and reported by [`load_errors`]. Nothing here panics on a hostile pack.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::lang::ProofLang;
use crate::patterns::Patterns;

/// The base name of a pack's Hunspell pair (`spelling.aff`, `spelling.dic`).
pub(crate) const SPELLING: &str = "spelling";
/// A pack's hyphenation patterns.
const HYPHENATION: &str = "hyphenation.pat";
/// Most languages whose pack patterns are remembered (a document may tag thousands).
const MAX_HYPHENATION_LOOKUPS: usize = 256;
/// The memory budget of the remembered pack patterns, in estimated bytes ([`pattern_bytes`]):
/// a pack's patterns may be as large as a dictionary (64 MB of file), and up to
/// [`MAX_HYPHENATION_LOOKUPS`] of them are kept, so they get a budget of their own, as the
/// dictionaries do (`pack_lang::MAX_LOADED_BYTES`).
const MAX_HYPHENATION_BYTES: u64 = 256 << 20;
/// Estimated memory per pattern (its packed key, its values and the map's own room).
const BYTES_PER_PATTERN: u64 = 64;
/// A pack's description.
const PACK_TOML: &str = "pack.toml";
/// A pack's licence texts.
const LICENCES: &str = "LICENCES";
/// Largest `pack.toml` read.
const MAX_PACK_TOML: u64 = 1 << 20;
/// Largest licence file read.
const MAX_LICENCE: u64 = 4 << 20;
/// Most licence files read from one pack.
const MAX_LICENCE_FILES: usize = 64;
/// Most packs read from the packs folder.
const MAX_PACKS: usize = 1024;
/// Longest locale tag.
const MAX_LOCALE: usize = 35;

/// An installed Language pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledPack {
    /// Lowercase BCP 47 tag (`nl`, `pt-br`), also the pack's folder name.
    pub locale: String,
    /// The language's own name (`Nederlands`); the locale when `pack.toml` gives none.
    pub name: String,
    /// SPDX licence expression of the pack's data.
    pub licence: String,
    /// The pack's folder.
    pub path: PathBuf,
}

/// What a scan of the packs folder found, with what each pack holds.
#[derive(Default)]
struct Scan {
    packs: Vec<(InstalledPack, Holds)>,
    /// (locale or folder name, reason) of each pack that was skipped.
    errors: Vec<(String, String)>,
}

#[derive(Clone, Copy, Default)]
struct Holds {
    spelling: bool,
    hyphenation: bool,
}

struct State {
    dir: Option<PathBuf>,
    scan: Option<Arc<Scan>>,
    /// Each language's pack patterns once looked up (`None`: no pack has them), at most
    /// [`MAX_HYPHENATION_LOOKUPS`] languages and [`MAX_HYPHENATION_BYTES`] (cleared when full).
    hyphenation: BTreeMap<ProofLang, Option<Arc<Patterns>>>,
    /// The estimated memory of the patterns in [`State::hyphenation`].
    hyphenation_bytes: u64,
    /// (locale, reason) of pack hyphenation files that failed to load.
    hyphenation_errors: Vec<(String, String)>,
    /// Counts changes, so a scan or load that started before one is not kept.
    changes: u64,
}

static STATE: RwLock<State> =
    RwLock::new(State { dir: None, scan: None, hyphenation: BTreeMap::new(), hyphenation_bytes: 0, hyphenation_errors: Vec::new(), changes: 0 });

fn read() -> RwLockReadGuard<'static, State> {
    STATE.read().unwrap_or_else(PoisonError::into_inner)
}

fn write() -> RwLockWriteGuard<'static, State> {
    STATE.write().unwrap_or_else(PoisonError::into_inner)
}

/// Use `dir` as the installed-packs folder (`<font-store folder>/language-packs`); `None`: no
/// packs. Every dictionary loads again and text is checked again
/// ([`crate::dictionary_generation`] changes), so call it again after a pack is installed or
/// removed (or call [`reload`]).
pub fn set_packs_dir(dir: Option<PathBuf>) {
    {
        let mut s = write();
        s.dir = dir;
        forget(&mut s);
    }
    crate::reload_dictionaries();
}

/// Read the packs folder again (a pack was installed or removed), as [`set_packs_dir`] with the
/// same folder.
pub fn reload() {
    forget(&mut write());
    crate::reload_dictionaries();
}

fn forget(s: &mut State) {
    s.scan = None;
    s.hyphenation.clear();
    s.hyphenation_bytes = 0;
    s.hyphenation_errors.clear();
    s.changes = s.changes.wrapping_add(1);
}

/// The installed-packs folder set with [`set_packs_dir`].
pub fn packs_dir() -> Option<PathBuf> {
    read().dir.clone()
}

/// The installed packs, by locale.
pub fn installed() -> Vec<InstalledPack> {
    scan().packs.iter().map(|(p, _)| p.clone()).collect()
}

/// Why each pack that couldn't be used was skipped or failed to load, as (locale or folder name,
/// reason): a missing or unreadable `pack.toml`, a path outside the pack, a hyphenation file that
/// isn't UTF-8 or is too large. A spelling dictionary that fails to load is reported by
/// [`crate::dictionary_load_error`] like any other.
pub fn load_errors() -> Vec<(String, String)> {
    let mut out = scan().errors.clone();
    out.extend(read().hyphenation_errors.iter().cloned());
    out
}

/// The licence texts of the Language data in use, as (title, full text): the English data
/// embedded at build time, then each installed pack's `LICENCES/` files (About › Language data).
pub fn licence_texts() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    // One Moby entry: the embedded data's, else the copy built into the crate (`en-words.dic`).
    if crate::data::EN_US_DIC.is_empty() {
        out.push(("English word list (Moby Hyphenator II)".into(), BUILT_IN_MOBY_NOTICE.into()));
    } else {
        out.push(("English word list and hyphenation (Moby Hyphenator II)".into(), MOBY_NOTICE.into()));
    }
    out.push(("English word supplement (en-extra.txt)".into(), EXTRA_EN_NOTICE.into()));
    out.extend(crate::data::EMBEDDED_LICENCES.iter().map(|(t, x)| ((*t).to_string(), (*x).to_string())));
    // Each file is checked against what is left before it is read, so no more is ever read.
    let mut left: u64 = MAX_LICENCE_TOTAL;
    for (p, _) in &scan().packs {
        for (file, text) in licence_files(&p.path, &mut left) {
            out.push((format!("{} ({}): {}", p.name, p.locale, plain_text(&file)), text));
        }
        if left == 0 {
            log::warn!("Language pack licences: {MAX_LICENCE_TOTAL} bytes in all; the rest aren't shown");
            break;
        }
    }
    out
}

/// Licence bytes read so far (tests check the cap holds while reading, not only after).
#[cfg(test)]
pub(crate) static LICENCE_BYTES_READ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Most bytes of pack licence text [`licence_texts`] reads in all.
const MAX_LICENCE_TOTAL: u64 = 16 << 20;

/// The notice of the embedded Moby word list (craft-i18n `language-data/en-us/en-us.dic`).
const MOBY_NOTICE: &str = "The Moby Hyphenator II word list by Grady Ward (Project Gutenberg eBook #3204) is in the public domain. \
The English hyphenation patterns were trained from it for the Craft apps and are distributed with them under MIT OR Apache-2.0.";

/// The notice of the word list built into the crate (`data/en-words.dic`).
const BUILT_IN_MOBY_NOTICE: &str = "The English word list built into craft-proof is the Moby Hyphenator II word list by Grady Ward \
(Project Gutenberg eBook #3204), which is in the public domain.";

/// The notice of our English supplement (`data/en-extra.txt`, and craft-i18n's copy).
const EXTRA_EN_NOTICE: &str = "The English word supplement (en-extra.txt), common words the Moby list lacks, is written by \
WordCraft contributors and licensed MIT OR Apache-2.0.";

/// The locales whose Language data is embedded in this build: `en-us`, and by default `de`,
/// `pl`, `pt`, `it`, `ru` (until Language packs ship, CP4.4); a build with `CRAFT_PROOF_EMBED`
/// embeds English and the ones it names (`en`: English only).
pub fn embedded_locales() -> &'static [&'static str] {
    crate::data::EMBEDDED_LOCALES
}

/// The scan of the packs folder, made on first use after each change.
fn scan() -> Arc<Scan> {
    let (dir, changes) = {
        let s = read();
        if let Some(scan) = &s.scan {
            return scan.clone();
        }
        (s.dir.clone(), s.changes)
    };
    let found = Arc::new(dir.as_deref().map(scan_dir).unwrap_or_default());
    let mut s = write();
    if s.changes != changes {
        // The folder changed meanwhile: this scan serves this call only.
        return found;
    }
    s.scan.get_or_insert(found).clone()
}

/// Read every pack in `dir` (sorted by folder name, at most [`MAX_PACKS`]).
fn scan_dir(dir: &Path) -> Scan {
    let mut scan = Scan::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            // No folder yet is no packs, not an error.
            if e.kind() != std::io::ErrorKind::NotFound {
                scan.errors.push((dir.display().to_string(), e.to_string()));
            }
            return scan;
        }
    };
    let mut names: Vec<String> = entries.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).take(MAX_PACKS * 2).collect();
    names.sort();
    for name in names.into_iter().filter(|n| !n.starts_with('.')).take(MAX_PACKS) {
        match read_pack(&dir.join(&name), &name) {
            Ok(found) => scan.packs.push(found),
            Err(e) => {
                log::warn!("Language pack {name}: {e}");
                scan.errors.push((name, e));
            }
        }
    }
    scan
}

/// One pack folder, named `folder`.
fn read_pack(path: &Path, folder: &str) -> Result<(InstalledPack, Holds), String> {
    if !is_locale(folder) {
        return Err("the folder name isn't a lowercase language tag".into());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir() {
        return Err("not a folder".into());
    }
    let toml = path.join(PACK_TOML);
    if !is_regular_file(&toml) {
        return Err("no pack.toml".into());
    }
    let text = String::from_utf8(read_capped(&toml, MAX_PACK_TOML)?).map_err(|_| "pack.toml: not UTF-8".to_string())?;
    let info = parse_pack_toml(&text)?;
    if info.locale != folder {
        return Err(format!("pack.toml names locale {:?}, not {folder:?}", info.locale));
    }
    let holds = Holds {
        spelling: is_regular_file(&path.join(format!("{SPELLING}.aff"))) && is_regular_file(&path.join(format!("{SPELLING}.dic"))),
        hyphenation: is_regular_file(&path.join(HYPHENATION)),
    };
    let name = info.name.unwrap_or_else(|| info.locale.clone());
    Ok((InstalledPack { locale: info.locale, name, licence: info.licence, path: path.to_path_buf() }, holds))
}

/// What `pack.toml` says.
struct PackInfo {
    locale: String,
    name: Option<String>,
    licence: String,
}

/// Parse `pack.toml`: `locale` and `licence` are required strings, `name` optional; anything else
/// is kept for people, but every value under a key that names a file (`path`, `file`, `files`,
/// or ending in `_path`, `_file`, `_files`), at any depth, must be a plain relative path inside
/// the pack.
fn parse_pack_toml(text: &str) -> Result<PackInfo, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| format!("pack.toml: {}", e.message()))?;
    check_paths(&toml::Value::Table(table.clone()), None, 0)?;
    let string = |key: &str| table.get(key).and_then(toml::Value::as_str).map(str::to_string);
    let locale = string("locale").ok_or("pack.toml: no locale")?;
    if !is_locale(&locale) {
        return Err(format!("pack.toml: {locale:?} isn't a lowercase language tag"));
    }
    let licence = string("licence").or_else(|| string("license")).map(|l| plain_text(&l)).filter(|l| !l.is_empty()).ok_or("pack.toml: no licence")?;
    let name = string("name").map(|n| plain_text(&n)).filter(|n| !n.is_empty());
    Ok(PackInfo { locale, name, licence })
}

/// Text from a pack shown to people (its name, licence and licence file names): without control, bidirectional or other format
/// characters (U+202E would reverse what follows), trimmed, at most 200 characters.
pub(crate) fn plain_text(s: &str) -> String {
    s.chars().filter(|&c| !c.is_control() && !is_format(c)).take(200).collect::<String>().trim().to_string()
}

/// Unicode format characters (general category Cf): bidirectional controls, zero-width ones,
/// the soft hyphen, tags.
fn is_format(c: char) -> bool {
    matches!(c,
        '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}' | '\u{890}'..='\u{891}' | '\u{8e2}' | '\u{180e}'
        | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{feff}'
        | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}' | '\u{13430}'..='\u{1343f}' | '\u{1bca0}'..='\u{1bca3}'
        | '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')
}

/// Deepest nesting of tables and arrays checked in `pack.toml` (deeper is refused).
const MAX_TOML_DEPTH: usize = 16;

/// Every string under a key that names a file is a [`safe_relative`] path.
fn check_paths(v: &toml::Value, key: Option<&str>, depth: usize) -> Result<(), String> {
    if depth > MAX_TOML_DEPTH {
        return Err("pack.toml: nested too deeply".into());
    }
    let names_file = key.is_some_and(|k| {
        let k = k.to_ascii_lowercase();
        ["path", "file", "files"].contains(&k.as_str()) || k.ends_with("_path") || k.ends_with("_file") || k.ends_with("_files")
    });
    match v {
        toml::Value::String(s) if names_file && !safe_relative(s) => {
            Err(format!("pack.toml: {:?} = {s:?} points outside the pack", key.unwrap_or_default()))
        }
        toml::Value::Table(t) => t.iter().try_for_each(|(k, v)| check_paths(v, Some(k), depth + 1)),
        toml::Value::Array(a) => a.iter().try_for_each(|v| check_paths(v, key, depth + 1)),
        _ => Ok(()),
    }
}

/// A relative path that stays inside the folder it is read from: no root, drive, `..`, `.`,
/// backslash, empty part or control character.
fn safe_relative(p: &str) -> bool {
    !p.is_empty()
        && !p.contains('\\')
        && !p.contains(':')
        && !p.chars().any(char::is_control)
        && p.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

/// A lowercase BCP 47 tag as packs are named: ASCII lowercase letters and digits in parts of 1 to
/// 8 joined by `-`, at most [`MAX_LOCALE`] characters.
fn is_locale(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_LOCALE
        && s.split('-').all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// A regular file, not a symbolic link (a pack never reads through one).
fn is_regular_file(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

/// A pack file: a regular file (not a symbolic link) of at most `cap` bytes.
fn read_capped(p: &Path, cap: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if !is_regular_file(p) {
        return Err(format!("{}: not a regular file", p.display()));
    }
    let err = |e: std::io::Error| format!("{}: {e}", p.display());
    let f = std::fs::File::open(p).map_err(err)?;
    let mut buf = Vec::new();
    f.take(cap.saturating_add(1)).read_to_end(&mut buf).map_err(err)?;
    if buf.len() as u64 > cap {
        return Err(format!("{}: too large (over {cap} bytes)", p.display()));
    }
    Ok(buf)
}

/// The licence texts in a pack's `LICENCES/` folder, by file name.
fn licence_files(pack: &Path, left: &mut u64) -> Vec<(String, String)> {
    let dir = pack.join(LICENCES);
    if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .take(MAX_LICENCE_FILES)
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|n| {
            let path = dir.join(&n);
            let size = std::fs::symlink_metadata(&path).ok().filter(std::fs::Metadata::is_file)?.len();
            if size > (*left).min(MAX_LICENCE) {
                // Too big, or over what is left: this file is skipped, smaller ones still fit.
                log::warn!("Language pack licence {} is {size} bytes; not shown", path.display());
                return None;
            }
            let bytes = read_capped(&path, size.min(MAX_LICENCE)).ok()?;
            *left = left.saturating_sub(bytes.len() as u64);
            #[cfg(test)]
            LICENCE_BYTES_READ.fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
            Some((n, String::from_utf8_lossy(&bytes).into_owned()))
        })
        .collect()
}

/// Lowercase pack locales for Hunspell dictionary names: `pt_BR` gives `pt-br`, then `pt`.
fn locales_for(names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |l: String| {
        if is_locale(&l) && !out.contains(&l) {
            out.push(l);
        }
    };
    for n in names {
        push(n.to_ascii_lowercase().replace('_', "-"));
    }
    // The primary language alone, when it is written in the same script (`sr_Latn` is never
    // plain `sr`, which is Cyrillic; `sr_Cyrl` is).
    for n in names {
        let primary = n.split(['_', '-']).next().unwrap_or_default().to_ascii_lowercase();
        if same_script(n, &primary) {
            push(primary);
        }
    }
    out
}

/// The locale a pack stands for: the bare `es` pack is Spain's (`es-es`) and the bare `de` pack
/// Germany's (`de-de`), as their sources are; any other pack is its locale.
fn pack_region(locale: &str) -> &str {
    match locale {
        "es" => "es-es",
        "de" => "de-de",
        l => l,
    }
}

/// Are two tags written in the same script ([`crate::lang::effective_script`])?
fn same_script(a: &str, b: &str) -> bool {
    crate::lang::effective_script(a) == crate::lang::effective_script(b)
}

/// The installed packs to read for `locales`, in that order, then any other pack of their
/// primary languages (`pt` → `pt-br`, `pt-pt`), keeping those that `has` the data.
fn packs_for(locales: &[String], has: impl Fn(&Holds) -> bool) -> Vec<PathBuf> {
    let scan = scan();
    let usable = || scan.packs.iter().filter(|(_, h)| has(h));
    let mut out: Vec<PathBuf> = Vec::new();
    for l in locales {
        if let Some((p, _)) = usable().find(|(p, _)| pack_region(&p.locale) == l) {
            out.push(p.path.clone());
        }
    }
    // Then another region's pack of the same language (`pt` → `pt-br`), never another script's.
    for (p, _) in usable() {
        let primary = p.locale.split('-').next().unwrap_or_default();
        let fits = locales.iter().any(|l| l.split('-').next() == Some(primary) && same_script(l, &p.locale));
        if fits && !out.contains(&p.path) {
            out.push(p.path.clone());
        }
    }
    out
}

/// The folders of the installed packs with a spelling dictionary for the Hunspell dictionary
/// `names` (`pt_BR`, `pt-BR`, `pt`), best first ([`crate::hunspell::find`] reads them), in two
/// tiers: the pack of the first name's own region ([`region_locales`]: `pt-br`), then every other
/// one (the other names' regions, then any other region of the language: `pt-pt`, `pt`).
/// The system's exact-region dictionaries are searched between the two, so another region's pack
/// never beats them.
pub(crate) fn spelling_dirs(names: &[&str]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    if read().dir.is_none() {
        return (Vec::new(), Vec::new());
    }
    let all = packs_for(&locales_for(names), |h| h.spelling);
    let region = region_locales(names);
    let scan = scan();
    let in_region = |path: &PathBuf| scan.packs.iter().any(|(p, _)| &p.path == path && region.iter().any(|r| r == pack_region(&p.locale)));
    all.into_iter().partition(in_region)
}

/// The folders of the installed packs with a spelling dictionary whose locale is exactly one of
/// `names` (`en_GB` → `en-gb`), in that order: no other region, no language alone.
pub(crate) fn exact_spelling_dirs(names: &[&str]) -> Vec<PathBuf> {
    if read().dir.is_none() {
        return Vec::new();
    }
    let scan = scan();
    let mut out: Vec<PathBuf> = Vec::new();
    for n in names {
        let l = n.to_ascii_lowercase().replace('_', "-");
        if let Some((p, _)) = scan.packs.iter().find(|(p, h)| h.spelling && pack_region(&p.locale) == l)
            && !out.contains(&p.path)
        {
            out.push(p.path.clone());
        }
    }
    out
}

/// The pack locales of `names`' own region: the first name's (`pt_BR` → `pt-br`, `es_ES` →
/// `es-es`, which the bare `es` pack is), and, when it names no region (`sr_Cyrl`), the language
/// alone (`sr`) if it is written in the same script. A region's language-alone pack (`fr` for
/// `fr_CA`) is another region's: it comes after the system's dictionary of the right region.
pub(crate) fn region_locales(names: &[&str]) -> Vec<String> {
    let Some(first) = names.first() else { return Vec::new() };
    let mut out = vec![first.to_ascii_lowercase().replace('_', "-")];
    let primary = first.split(['_', '-']).next().unwrap_or_default().to_ascii_lowercase();
    if crate::lang::region(first).is_none() && same_script(first, &primary) && !out.contains(&primary) {
        out.push(primary);
    }
    out.retain(|l| is_locale(l));
    out
}

/// The pack locales whose patterns hyphenate `lang`, best first; none for English (embedded)
/// and languages without proofing.
fn hyphenation_locales(lang: ProofLang) -> Vec<String> {
    let names: &[&str] = match lang {
        ProofLang::Pl => &["pl"],
        ProofLang::Bg => &["bg"],
        ProofLang::Pt => &["pt"],
        ProofLang::It => &["it"],
        ProofLang::Hr => &["hr"],
        ProofLang::Tr => &["tr"],
        ProofLang::Ru => &["ru"],
        ProofLang::Cs => &["cs"],
        ProofLang::Id => &["id"],
        ProofLang::Es => &["es-es", "es"],
        ProofLang::EsMx => &["es-mx", "es-419", "es"],
        ProofLang::Es419 => &["es-419", "es"],
        ProofLang::EsUy => &["es-uy", "es-419", "es"],
        ProofLang::De => &["de", "de-1996", "de-de"],
        ProofLang::Pack(l) => return crate::pack_lang::hyphenation_locales(l),
        ProofLang::En | ProofLang::EnGb | ProofLang::EnAu | ProofLang::EnCa | ProofLang::Other => &[],
    };
    names.iter().map(|n| (*n).to_string()).collect()
}

/// The hyphenation patterns of an installed pack for `lang`, if there is one (read on first use
/// after each change). A pack file that fails to load is reported by [`load_errors`] and the next
/// pack (or the embedded patterns) is used.
pub(crate) fn hyphenation(lang: ProofLang) -> Option<Arc<Patterns>> {
    let changes = {
        let s = read();
        s.dir.as_ref()?;
        if let Some(p) = s.hyphenation.get(&lang) {
            return p.clone();
        }
        s.changes
    };
    let mut errors = Vec::new();
    let mut found = None;
    for dir in packs_for(&hyphenation_locales(lang), |h| h.hyphenation) {
        match load_patterns(&dir.join(HYPHENATION)) {
            Ok(p) => {
                found = Some(Arc::new(p));
                break;
            }
            Err(e) => {
                let locale = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                log::warn!("Language pack {locale}: {e}");
                errors.push((locale, e));
            }
        }
    }
    let mut s = write();
    if s.changes == changes && !s.hyphenation.contains_key(&lang) {
        // ponytail: cleared when full, an LRU if many languages each need their patterns.
        let bytes = found.as_deref().map_or(0, pattern_bytes);
        remember_patterns(&mut s, lang, found.clone(), bytes, MAX_HYPHENATION_BYTES);
        for e in errors {
            if !s.hyphenation_errors.contains(&e) {
                s.hyphenation_errors.push(e);
            }
        }
    }
    found
}

/// Estimated memory of `p`.
fn pattern_bytes(p: &Patterns) -> u64 {
    (p.len() as u64).saturating_add(p.exception_count() as u64).saturating_mul(BYTES_PER_PATTERN)
}

/// Remember `lang`'s pack patterns (`bytes` of them): over [`MAX_HYPHENATION_LOOKUPS`] languages
/// or `budget` bytes, the ones remembered so far are forgotten first. Patterns larger than the
/// whole budget are still kept alone, or they would be read from disk for every word.
fn remember_patterns(s: &mut State, lang: ProofLang, found: Option<Arc<Patterns>>, bytes: u64, budget: u64) {
    if s.hyphenation.len() >= MAX_HYPHENATION_LOOKUPS || s.hyphenation_bytes.saturating_add(bytes) > budget {
        s.hyphenation.clear();
        s.hyphenation_bytes = 0;
    }
    s.hyphenation.insert(lang, found);
    s.hyphenation_bytes = s.hyphenation_bytes.saturating_add(bytes);
}

/// How many languages' pack patterns are remembered now.
#[cfg(test)]
pub(crate) fn hyphenation_lookups() -> usize {
    read().hyphenation.len()
}

/// A pack's `hyphenation.pat` (UTF-8 TeX or plain Liang patterns, at most the dictionary cap).
fn load_patterns(path: &Path) -> Result<Patterns, String> {
    let bytes = read_capped(path, crate::hunspell::MAX_FILE)?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", path.display()))?;
    let p = Patterns::parse(&text);
    if p.is_empty() {
        return Err(format!("{}: no patterns", path.display()));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review follow-up (craft-libs#485): the remembered pack patterns have a memory budget, not
    /// only a count, so 256 languages of large patterns can't each stay in memory.
    #[test]
    fn remembered_patterns_keep_to_a_memory_budget() {
        let mut s = State { dir: None, scan: None, hyphenation: BTreeMap::new(), hyphenation_bytes: 0, hyphenation_errors: Vec::new(), changes: 0 };
        let p = Arc::new(Patterns::parse("hy3ph he2n hena4 hen5at 1na n2at 1tio 2io o2n"));
        let bytes = pattern_bytes(&p);
        assert_eq!(bytes, 9 * BYTES_PER_PATTERN);
        let lang = |t: &str| ProofLang::from_tag(Some(t));
        let budget = bytes * 2;
        remember_patterns(&mut s, lang("nl"), Some(p.clone()), bytes, budget);
        remember_patterns(&mut s, lang("fr"), Some(p.clone()), bytes, budget);
        remember_patterns(&mut s, lang("sv"), None, 0, budget);
        assert_eq!((s.hyphenation.len(), s.hyphenation_bytes), (3, budget), "a language with no patterns takes no room");
        remember_patterns(&mut s, lang("da"), Some(p.clone()), bytes, budget);
        assert_eq!((s.hyphenation.len(), s.hyphenation_bytes), (1, bytes), "over the budget: the others are forgotten");
        assert!(s.hyphenation.contains_key(&lang("da")));
        remember_patterns(&mut s, lang("fi"), Some(p.clone()), budget * 4, budget);
        assert_eq!((s.hyphenation.len(), s.hyphenation_bytes), (1, budget * 4), "patterns over the whole budget are kept alone");
        forget(&mut s);
        assert_eq!((s.hyphenation.len(), s.hyphenation_bytes), (0, 0));
    }

    /// Review of #485: a pack's name loses bidirectional and other format characters.
    #[test]
    fn pack_names_are_plain_text() {
        let t = "locale = \"nl\"\nlicence = \"MIT\"\nname = \"Neder\\u202elands\\u200b\\u2066 \\u0007\"\n";
        assert_eq!(parse_pack_toml(t).unwrap().name.as_deref(), Some("Nederlands"));
        let t = "locale = \"nl\"\nlicence = \"MIT\"\nname = \"\\u202e\\u200f\"\n";
        assert_eq!(parse_pack_toml(t).unwrap().name, None, "nothing left: the locale is shown");
        let t = "locale = \"nl\"\nlicence = \"MIT\\u202e LPG\"\n";
        assert_eq!(parse_pack_toml(t).unwrap().licence, "MIT LPG");
        let t = "locale = \"nl\"\nlicence = \"\\u200f\"\n";
        assert!(parse_pack_toml(t).is_err(), "no licence left");
    }

    #[test]
    fn locales_are_lowercase_tags() {
        for ok in ["nl", "pt-br", "sr-latn", "es-419", "de-1996", "ca-valencia"] {
            assert!(is_locale(ok), "{ok}");
        }
        for bad in ["", "NL", "pt_BR", "..", "a/b", "-nl", "nl-", "nl--x", "abcdefghi", "x\u{0}", "é", &"a-".repeat(20)] {
            assert!(!is_locale(bad), "{bad:?}");
        }
    }

    #[test]
    fn dictionary_names_map_to_pack_locales() {
        assert_eq!(locales_for(&["pt_BR", "pt-BR", "pt"]), ["pt-br", "pt"]);
        assert_eq!(region_locales(&["pt_BR", "pt-BR", "pt"]), ["pt-br"]);
        assert_eq!(region_locales(&["en_US", "en-US", "en", "en_GB"]), ["en-us"]);
        assert_eq!(region_locales(&["sr_Cyrl", "sr-Cyrl"]), ["sr-cyrl", "sr"]);
        assert_eq!(region_locales(&["nl"]), ["nl"]);
        assert_eq!(region_locales(&["sr_Latn", "sr-Latn"]), ["sr-latn"]);
        assert!(region_locales(&[]).is_empty());
        assert_eq!(locales_for(&["es_419", "es"]), ["es-419", "es"]);
        assert_eq!(locales_for(&["../x", "a b"]), Vec::<String>::new());
    }

    #[test]
    fn pack_toml_paths_stay_inside_the_pack() {
        let ok = "locale = \"nl\"\nlicence = \"BSD-3-Clause\"\nname = \"Nederlands\"\nsources = [\"https://example.org/nl.aff\"]\n[[file]]\npath = \"LICENCES/COPYING\"\n";
        let info = parse_pack_toml(ok).unwrap();
        assert_eq!((info.locale.as_str(), info.name.as_deref(), info.licence.as_str()), ("nl", Some("Nederlands"), "BSD-3-Clause"));
        for bad in ["../../etc/passwd", "/etc/passwd", "C:/x", "LICENCES/../../x", "a\\b", "", "./x"] {
            let t = format!("locale = \"nl\"\nlicence = \"MIT\"\n[[file]]\npath = {bad:?}\n");
            assert!(parse_pack_toml(&t).is_err(), "{bad:?}");
            let t = format!("locale = \"nl\"\nlicence = \"MIT\"\nlicence_files = [{bad:?}]\n");
            assert!(parse_pack_toml(&t).is_err(), "{bad:?}");
        }
        for bad in [
            "licence = \"MIT\"\n",
            "locale = \"nl\"\n",
            "locale = \"../nl\"\nlicence = \"MIT\"\n",
            "locale = \"nl\"\nlicence = \"MIT\"\nlocale = \"x\"",
            "\u{0}",
        ] {
            assert!(parse_pack_toml(bad).is_err(), "{bad:?}");
        }
        let deep = format!("locale = \"nl\"\nlicence = \"MIT\"\nx = {}{}\n", "[".repeat(40), "]".repeat(40));
        assert!(parse_pack_toml(&deep).is_err());
    }

    /// Review of #485: our own word supplements (`ru-extra.txt`, MIT) stay embedded in every
    /// build, the English-only one too.
    #[test]
    fn our_supplements_are_always_embedded() {
        if cfg!(craft_proof_data) {
            assert!(!crate::data::EXTRA_RU.is_empty(), "ru-extra.txt is embedded");
        }
    }

    /// #5208 review: until Language packs are published, the default build embeds the
    /// carry-over locales (de, pl, pt, it patterns, ru-extra); `CRAFT_PROOF_EMBED=en` is the
    /// English-only build (CP4.4's default once language-packs-1 is pinned).
    #[test]
    fn the_default_build_embeds_the_carry_over_locales() {
        let embed = option_env!("CRAFT_PROOF_EMBED").unwrap_or_default().trim().to_ascii_lowercase();
        let named: Vec<&str> = if embed.is_empty() { vec!["de", "pl", "pt", "it", "ru"] } else { embed.split(',').map(str::trim).collect() };
        for l in embedded_locales() {
            assert!(l.starts_with("en-") || *l == "ru" || named.iter().any(|e| e == l || *e == "all"), "{l} is embedded");
        }
        if cfg!(craft_proof_data) && embed.is_empty() {
            for l in ["de", "pl", "pt", "it"] {
                assert!(embedded_locales().contains(&l), "{l} is embedded by default");
            }
        }
    }

    /// One Moby entry, and the supplement's own licence.
    #[test]
    fn built_in_notices_name_the_real_sources() {
        let texts = licence_texts();
        assert_eq!(texts.iter().filter(|(t, _)| t.contains("Moby")).count(), 1, "{texts:?}");
        assert!(texts.iter().any(|(t, x)| t.contains("en-extra.txt") && x.contains("MIT OR Apache-2.0")));
        assert!(!texts.iter().any(|(t, _)| t.contains("core")), "the core list is gone");
    }
}
