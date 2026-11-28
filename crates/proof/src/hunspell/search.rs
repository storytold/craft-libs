//! Where installed Hunspell dictionaries are looked for, and why a found one failed to load.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// An extra directory searched first (a setting), set by the front end.
static EXTRA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Why an installed dictionary failed to load, by `.dic` name (`pt_BR`): the file and the reason.
/// Without it a file that is there but unreadable looks the same as no file (#2830).
static LOAD_ERRORS: RwLock<Vec<(String, PathBuf, String)>> = RwLock::new(Vec::new());

/// Remember (or, for `Ok`, forget) why the dictionary `key` found at `dic` failed to load, and
/// hand the result back unchanged.
pub fn note_load<T>(key: &str, dic: &Path, result: Result<T, String>) -> Result<T, String> {
    let mut errors = LOAD_ERRORS.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    errors.retain(|(k, _, _)| k != key);
    if let Err(e) = &result {
        errors.push((key.to_string(), dic.to_path_buf(), e.clone()));
    }
    result
}

/// Forget the load error of the dictionary `key` (none found, or it is about to be searched for again).
pub fn clear_load_error(key: &str) {
    LOAD_ERRORS.write().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|(k, _, _)| k != key);
}

/// The `.dic` file and the reason of the last failed load of the dictionary `key`, if any.
pub fn load_error(key: &str) -> Option<(PathBuf, String)> {
    LOAD_ERRORS.read().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(k, _, _)| k == key).map(|(_, p, e)| (p.clone(), e.clone()))
}

/// Search `dir` before the system locations (a "dictionaries folder" setting); `None` clears it.
/// Takes effect for dictionaries not loaded yet.
pub fn set_dictionary_dir(dir: Option<PathBuf>) {
    *EXTRA_DIR.write().unwrap_or_else(std::sync::PoisonError::into_inner) = dir;
    LOAD_ERRORS.write().unwrap_or_else(std::sync::PoisonError::into_inner).clear(); // they belong to the old search path
}

/// The LibreOffice extension directories a Hunspell dictionary name may live in: LibreOffice's
/// IDs are per dictionary — Polish is `dict-pl`, Brazilian Portuguese `dict-pt-BR` — so both
/// the primary-language folder and the dashed full name are tried.
pub(super) fn lo_ext_dirs(name: &str) -> Vec<String> {
    let primary = name.split(['-', '_']).next().unwrap_or(name);
    let dashed = name.replace('_', "-");
    let first = format!("dict-{primary}");
    let full = format!("dict-{dashed}");
    if full == first { vec![first] } else { vec![first, full] }
}

/// Directories that may hold Hunspell dictionaries, most specific first. `lo_exts`, when a
/// language is being searched for, adds LibreOffice's extension folders for it (`dict-pt-BR`).
pub fn search_dirs(lo_exts: Option<&[String]>) -> Vec<PathBuf> {
    let (mut dirs, system) = search_tiers(lo_exts);
    dirs.extend(system);
    dirs
}

/// [`search_dirs`] in two tiers: the dictionaries folders (the one set with
/// [`set_dictionary_dir`], `WORDCRAFT_DICTIONARIES` and WordCraft's own folder under the user's
/// application data), then the system's Hunspell and LibreOffice folders. Installed Language
/// packs ([`crate::packs`]) are searched between the two.
fn search_tiers(lo_exts: Option<&[String]>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut system: Vec<PathBuf> = Vec::new();
    if let Some(d) = EXTRA_DIR.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone() {
        dirs.push(d);
    }
    if cfg!(target_arch = "wasm32") {
        return (dirs, system);
    }
    if let Some(d) = std::env::var_os("WORDCRAFT_DICTIONARIES").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(d));
    }
    // LibreOffice keeps each language under `…/share/extensions/<dict-xx>`.
    let lo_dirs =
        |prefix: &str| -> Vec<PathBuf> { lo_exts.map_or_else(Vec::new, |exts| exts.iter().map(|e| PathBuf::from(prefix).join(e)).collect()) };
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        for var in ["LOCALAPPDATA", "APPDATA"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                dirs.push(base.join("WordCraft").join("dictionaries"));
                system.push(base.join("hunspell"));
            }
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                system.extend(lo_dirs(&base.join("LibreOffice").join("share").join("extensions").to_string_lossy()));
            }
        }
        system.extend(lo_drive_dirs(lo_exts, || LO_DRIVE_BASES.get_or_init(|| lo_drive_bases(Path::is_dir))));
    } else if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            // An app started from Finder or the Dock doesn't see `WORDCRAFT_DICTIONARIES`; this
            // folder needs no setting (#1063).
            dirs.push(h.join("Library/Application Support/WordCraft/dictionaries"));
            system.push(h.join("Library/Spelling"));
        }
        system.push(PathBuf::from("/Library/Spelling"));
        system.push(PathBuf::from("/opt/homebrew/share/hunspell"));
        system.push(PathBuf::from("/usr/local/share/hunspell"));
        system.extend(lo_dirs("/Applications/LibreOffice.app/Contents/Resources/extensions"));
    } else {
        // Nor does one started from a desktop launcher (#1063).
        if let Some(data) = xdg_data_home(std::env::var_os("XDG_DATA_HOME"), home.as_deref()) {
            dirs.push(data.join("wordcraft").join("dictionaries"));
        }
        if let Some(h) = &home {
            system.push(h.join(".local/share/hunspell"));
        }
        for d in ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell", "/usr/local/share/myspell"] {
            system.push(PathBuf::from(d));
        }
        for d in ["/usr/lib/libreoffice/share/extensions", "/opt/libreoffice/share/extensions"] {
            system.extend(lo_dirs(d));
        }
    }
    (dirs, system)
}

/// LibreOffice's language folders (`<drive>:\Program Files\LibreOffice\share\extensions\<ext>`)
/// on the drives C to Z where LibreOffice is installed (`bases`). No drive is probed without
/// `lo_exts` (no LibreOffice folders are wanted then).
pub(super) fn lo_drive_dirs<'a>(lo_exts: Option<&[String]>, bases: impl FnOnce() -> &'a [PathBuf]) -> Vec<PathBuf> {
    let Some(exts) = lo_exts else { return Vec::new() };
    bases().iter().flat_map(|base| exts.iter().map(|e| base.join(e))).collect()
}

/// The `LibreOffice\share\extensions` folders on drives C to Z (48 probes).
pub(super) fn lo_drive_bases(is_dir: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    ('C'..='Z')
        .flat_map(|drive| ["Program Files", "Program Files (x86)"].map(|pf| PathBuf::from(format!(r"{drive}:\{pf}\LibreOffice\share\extensions"))))
        .filter(|base| is_dir(base))
        .collect()
}

/// [`lo_drive_bases`], probed once per process.
// ponytail: a LibreOffice installed while the app runs is seen after a restart.
static LO_DRIVE_BASES: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();

/// The user's data folder on Linux and the BSDs: `$XDG_DATA_HOME` when it is an absolute path (the
/// XDG Base Directory spec ignores a relative one), else `~/.local/share`.
pub(super) fn xdg_data_home(xdg: Option<std::ffi::OsString>, home: Option<&Path>) -> Option<PathBuf> {
    xdg.map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home.map(|h| h.join(".local/share")))
}

/// The first `<name>.aff`/`<name>.dic` pair found for any of `names` (`pl_PL`, `pl`), in the
/// order of [`candidates`].
pub fn find(names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    candidates(names).into_iter().next()
}

/// Every `<name>.aff`/`<name>.dic` pair for `names`, best first. A caller that can't read one
/// tries the next. The order:
///
/// 1. the dictionaries folders ([`set_dictionary_dir`] and the others of [`search_dirs`]), any
///    of `names`;
/// 2. an installed Language pack of the first name's region ([`crate::packs`],
///    `spelling.aff`/`spelling.dic`; the bare `es` and `de` packs are Spain's and Germany's);
/// 3. the system's folders, the first name's region (`pt_BR`, `pt-BR`);
/// 4. any other installed pack of the language (another region: `pt-pt`, the bare `pt`);
/// 5. the system's folders, the other names (`pt`).
pub fn candidates(names: &[&str]) -> Vec<(PathBuf, PathBuf)> {
    let lo = names.first().copied().map(lo_ext_dirs);
    let (dirs, system) = search_tiers(lo.as_deref());
    let (region_packs, other_packs) = crate::packs::spelling_dirs(names);
    ordered(names, &dirs, &region_packs, &system, &other_packs)
}

/// [`candidates`] over the given folders.
pub(crate) fn ordered(
    names: &[&str],
    dirs: &[PathBuf],
    region_packs: &[PathBuf],
    system: &[PathBuf],
    other_packs: &[PathBuf],
) -> Vec<(PathBuf, PathBuf)> {
    let norm = |n: &str| n.to_ascii_lowercase().replace('_', "-");
    let first = names.first().map(|n| norm(n));
    let (region, rest): (Vec<&str>, Vec<&str>) = names.iter().copied().partition(|n| Some(norm(n)) == first);
    let pack: &[&str] = &[crate::packs::SPELLING];
    // (folder, names, a Language pack's: never read through a symbolic link, as the pack scan
    // checks; a system folder's dictionaries often are links, `en_GB.dic` → `en_GB-ise.dic`).
    let tiers = dirs
        .iter()
        .map(|d| (d, names, false))
        .chain(region_packs.iter().map(|d| (d, pack, true)))
        .chain(system.iter().map(|d| (d, &region[..], false)))
        .chain(other_packs.iter().map(|d| (d, pack, true)))
        .chain(system.iter().map(|d| (d, &rest[..], false)));
    let file = |p: &Path, pack: bool| if pack { std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()) } else { p.is_file() };
    let mut out: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (dir, names, in_pack) in tiers {
        for n in names {
            let aff = dir.join(format!("{n}.aff"));
            let dic = dir.join(format!("{n}.dic"));
            if file(&aff, in_pack) && file(&dic, in_pack) && !out.iter().any(|(_, d)| *d == dic) {
                out.push((aff, dic));
            }
        }
    }
    out
}

/// [`candidates`] without another region's or dialect's dictionary: the dictionaries folders,
/// then an installed pack named exactly by one of `names` (`en_GB` → `en-gb`), then the system's
/// folders, `names` only. English uses it: en-GB never falls back to an `en_US` file or pack.
pub fn candidates_exact(names: &[&str]) -> Vec<(PathBuf, PathBuf)> {
    let lo = names.first().copied().map(lo_ext_dirs);
    let (dirs, system) = search_tiers(lo.as_deref());
    ordered(names, &dirs, &crate::packs::exact_spelling_dirs(names), &system, &[])
}

/// [`candidates`] in the folders the user keeps dictionaries in for WordCraft ([`user_dirs`])
/// only, not the system locations.
pub fn user_candidates(names: &[&str]) -> Vec<(PathBuf, PathBuf)> {
    candidates_in(&user_dirs(), names)
}

/// The folders of [`search_dirs`] the user keeps dictionaries in for WordCraft: the one set with
/// [`set_dictionary_dir`], `WORDCRAFT_DICTIONARIES`, and WordCraft's own `dictionaries` folder
/// under the user's application data. The rest are the system's (and LibreOffice's).
pub fn user_dirs() -> Vec<PathBuf> {
    let extra = EXTRA_DIR.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    let env = std::env::var_os("WORDCRAFT_DICTIONARIES").filter(|v| !v.is_empty()).map(PathBuf::from);
    let wordcraft_own = |d: &Path| {
        d.file_name().is_some_and(|n| n == "dictionaries")
            && d.parent().and_then(Path::file_name).is_some_and(|n| n.eq_ignore_ascii_case("wordcraft"))
    };
    search_dirs(None).into_iter().filter(|d| Some(d) == extra.as_ref() || Some(d) == env.as_ref() || wordcraft_own(d)).collect()
}

/// [`candidates`] in `dirs`.
pub fn candidates_in(dirs: &[PathBuf], names: &[&str]) -> Vec<(PathBuf, PathBuf)> {
    let mut out: Vec<(PathBuf, PathBuf)> = Vec::new();
    for n in names {
        for dir in dirs {
            let aff = dir.join(format!("{n}.aff"));
            let dic = dir.join(format!("{n}.dic"));
            if aff.is_file() && dic.is_file() && !out.iter().any(|(_, d)| *d == dic) {
                out.push((aff, dic));
            }
        }
    }
    out
}

/// The first `<name>.aff`/`<name>.dic` pair for any of `names` in `dir`.
pub fn find_in(dir: &Path, names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    names.iter().find_map(|n| {
        let aff = dir.join(format!("{n}.aff"));
        let dic = dir.join(format!("{n}.dic"));
        (aff.is_file() && dic.is_file()).then_some((aff, dic))
    })
}
