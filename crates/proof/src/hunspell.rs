//! Hunspell dictionaries installed on the system, read at runtime.
//!
//! The Craft apps don't bundle large spelling dictionaries. For a language with an affix
//! dictionary, an app reads the standard Hunspell `.aff`/`.dic` pair installed by the operating
//! system, a package manager or LibreOffice, with [`load`]. When none is installed the language
//! has no spelling check (no marks, no errors), like Word without the language's proofing tools.
//!
//! Where to look is per app ([`Search`]): its own dictionaries folder (a setting), an environment
//! variable and a folder under the user's application data (on every desktop system), then the
//! usual system locations. On the web there are none.

use std::path::{Path, PathBuf};

use crate::affix::AffixDict;

/// Largest `.aff` or `.dic` file read (real ones are a few MB).
const MAX_FILE: u64 = 64 << 20;

/// Where one app looks for Hunspell dictionaries, most specific first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    /// A folder searched before everything else, such as a "dictionaries folder" setting.
    pub extra_dir: Option<PathBuf>,
    /// An environment variable naming a folder searched next (`WORDCRAFT_DICTIONARIES`).
    pub env_var: Option<String>,
    /// The app's folder name under the user's application data. `WordCraft` searches:
    /// - Windows: `%LOCALAPPDATA%\WordCraft\dictionaries`, then `%APPDATA%\WordCraft\dictionaries`
    /// - macOS: `~/Library/Application Support/WordCraft/dictionaries`
    /// - Linux and the BSDs: `$XDG_DATA_HOME/wordcraft/dictionaries` (default
    ///   `~/.local/share/wordcraft/dictionaries`; the name is lower-cased there, as XDG folders are)
    ///
    /// An app started from Finder, the Dock or a desktop launcher doesn't see shell variables such
    /// as `WORDCRAFT_DICTIONARIES`, so this folder lets users add a dictionary without any setting.
    pub app_dir: Option<String>,
}

impl Search {
    /// The search for an app called `app` (`WordCraft`): its environment variable is the
    /// upper-cased name with `_DICTIONARIES` (`WORDCRAFT_DICTIONARIES`) and its folder under the
    /// user's application data the name itself (see [`Search::app_dir`]).
    pub fn for_app(app: &str) -> Search {
        Search { extra_dir: None, env_var: Some(format!("{}_DICTIONARIES", app.to_ascii_uppercase())), app_dir: Some(app.to_string()) }
    }

    /// Directories that may hold Hunspell dictionaries, most specific first. `lo_exts`, when a
    /// language is being searched for, adds LibreOffice's extension folders for it (`dict-pt-BR`).
    pub fn dirs(&self, lo_exts: Option<&[String]>) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self.extra_dir.iter().cloned().collect();
        if cfg!(target_arch = "wasm32") {
            return dirs;
        }
        if let Some(d) = self.env_var.as_deref().and_then(std::env::var_os).filter(|v| !v.is_empty()) {
            dirs.push(PathBuf::from(d));
        }
        // LibreOffice keeps each language under `…/share/extensions/<dict-xx>`.
        let lo_dirs = |base: &Path| -> Vec<PathBuf> { lo_exts.map_or_else(Vec::new, |exts| exts.iter().map(|e| base.join(e)).collect()) };
        let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
        if cfg!(windows) {
            let bases: Vec<PathBuf> = ["LOCALAPPDATA", "APPDATA"]
                .into_iter()
                .filter_map(|var| std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from))
                .collect();
            dirs.extend(windows_user_dirs(&bases, self.app_dir.as_deref()));
            for var in ["ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(base) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                    dirs.extend(lo_dirs(&base.join("LibreOffice").join("share").join("extensions")));
                }
            }
        } else if cfg!(target_os = "macos") {
            if let Some(h) = &home {
                if let Some(app) = &self.app_dir {
                    dirs.push(h.join("Library/Application Support").join(app).join("dictionaries"));
                }
                dirs.push(h.join("Library/Spelling"));
            }
            dirs.push(PathBuf::from("/Library/Spelling"));
            dirs.push(PathBuf::from("/opt/homebrew/share/hunspell"));
            dirs.push(PathBuf::from("/usr/local/share/hunspell"));
            dirs.extend(lo_dirs(Path::new("/Applications/LibreOffice.app/Contents/Resources/extensions")));
        } else {
            // XDG_DATA_HOME must be absolute to count; otherwise the spec's default applies.
            let data_home = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
            if let (Some(data), Some(app)) = (&data_home, &self.app_dir) {
                dirs.push(data.join(app.to_lowercase()).join("dictionaries"));
            }
            if let Some(h) = &home {
                dirs.push(h.join(".local/share/hunspell"));
            }
            for d in
                ["/usr/share/hunspell", "/usr/share/myspell", "/usr/share/myspell/dicts", "/usr/local/share/hunspell", "/usr/local/share/myspell"]
            {
                dirs.push(PathBuf::from(d));
            }
            for d in ["/usr/lib/libreoffice/share/extensions", "/opt/libreoffice/share/extensions"] {
                dirs.extend(lo_dirs(Path::new(d)));
            }
        }
        dirs
    }

    /// The first `<name>.aff`/`<name>.dic` pair found for any of `names` (`pl_PL`, `pl`),
    /// searching [`Search::dirs`] (including LibreOffice's extension folders for the first name).
    pub fn find(&self, names: &[&str]) -> Option<(PathBuf, PathBuf)> {
        let lo = names.first().copied().map(lo_ext_dirs);
        self.dirs(lo.as_deref()).into_iter().find_map(|dir| find_in(&dir, names))
    }
}

/// The LibreOffice extension directories a Hunspell dictionary name may live in: LibreOffice's
/// IDs are per dictionary — Polish is `dict-pl`, Brazilian Portuguese `dict-pt-BR` — so both
/// the primary-language folder and the dashed full name are tried.
fn lo_ext_dirs(name: &str) -> Vec<String> {
    let primary = name.split(['-', '_']).next().unwrap_or(name);
    let dashed = name.replace('_', "-");
    let first = format!("dict-{primary}");
    let full = format!("dict-{dashed}");
    if full == first { vec![first] } else { vec![first, full] }
}

/// The first `<name>.aff`/`<name>.dic` pair for any of `names` in `dir`.
pub fn find_in(dir: &Path, names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    names.iter().find_map(|n| {
        let aff = dir.join(format!("{n}.aff"));
        let dic = dir.join(format!("{n}.dic"));
        (aff.is_file() && dic.is_file()).then_some((aff, dic))
    })
}

/// Read a Hunspell pair from disk (size-capped), decoded per the `.aff`'s `SET` line.
pub fn load(aff: &Path, dic: &Path) -> Result<AffixDict, String> {
    let read = |p: &Path| -> Result<Vec<u8>, String> {
        let len = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?.len();
        if len > MAX_FILE {
            return Err(format!("{}: too large ({len} bytes)", p.display()));
        }
        std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let (aff_text, dic_text) = decode(&read(aff)?, &read(dic)?)?;
    let source = format!("Hunspell dictionary {}", dic.display());
    AffixDict::from_hunspell(&aff_text, &dic_text, &[source.as_str()])
}

/// ISO 8859-1: every byte is the code point of the same value (the original VERO pt_BR uses it).
fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// ISO 8859-2 code points 0xA0..=0xFF (0x00..=0x9F map to themselves).
const ISO_8859_2_HIGH: &str = "\u{a0}Ą˘Ł¤ĽŚ§¨ŠŞŤŹ\u{ad}ŽŻ°ą˛ł´ľśˇ¸šşťź˝žżŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢßŕáâăäĺćçčéęëěíîďđńňóôőö÷řůúűüýţ˙";

fn decode_latin2(bytes: &[u8]) -> String {
    let high: Vec<char> = ISO_8859_2_HIGH.chars().collect();
    bytes.iter().map(|&b| if b < 0xa0 { char::from(b) } else { high.get(usize::from(b - 0xa0)).copied().unwrap_or('\u{fffd}') }).collect()
}

/// KOI8-R code points 0x80..=0xFF (0x00..=0x7F are ASCII): the encoding of older Russian
/// dictionaries.
const KOI8_R_HIGH: &str =
    "─│┌┐└┘├┤┬┴┼▀▄█▌▐░▒▓⌠■∙√≈≤≥\u{a0}⌡°²·÷═║╒ё╓╔╕╖╗╘╙╚╛╜╝╞╟╠╡Ё╢╣╤╥╦╧╨╩╪╫╬©юабцдефгхийклмнопярстужвьызшэщчъЮАБЦДЕФГХИЙКЛМНОПЯРСТУЖВЬЫЗШЭЩЧЪ";

/// Windows-1251 code points 0x80..=0xFF (0x98 is unassigned): the other one Russian dictionaries
/// use.
const CP1251_HIGH: &str = "ЂЃ‚ѓ„…†‡€‰Љ‹ЊЌЋЏђ‘’“”•–—\u{fffd}™љ›њќћџ\u{a0}ЎўЈ¤Ґ¦§Ё©Є«¬\u{ad}®Ї°±Ііґµ¶·ё№є»јЅѕїАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдежзийклмнопрстуфхцчшщъыьэюя";

/// Decode a single-byte encoding whose bytes 0x80..=0xFF are `high` (ASCII below).
fn decode_high(bytes: &[u8], high: &str) -> String {
    let high: Vec<char> = high.chars().collect();
    bytes.iter().map(|&b| if b < 0x80 { char::from(b) } else { high.get(usize::from(b - 0x80)).copied().unwrap_or('\u{fffd}') }).collect()
}

/// Decode an `.aff`/`.dic` pair to UTF-8 from the encoding the `.aff`'s `SET` line names
/// (UTF-8 when there is none; ISO 8859-2 is the other one Polish dictionaries use, ISO 8859-1 the
/// one the original Portuguese VERO uses, KOI8-R and Windows-1251 the ones older Russian dictionaries use).
pub fn decode(aff: &[u8], dic: &[u8]) -> Result<(String, String), String> {
    let head = String::from_utf8_lossy(aff.get(..aff.len().min(8192)).unwrap_or_default()).to_string();
    // Only the first token names the charset; a trailing comment or field is ignored, as the
    // affix parser does for every directive.
    let set = head
        .lines()
        .find_map(|l| {
            let mut f = l.trim_start_matches('\u{feff}').split_whitespace();
            (f.next() == Some("SET")).then(|| f.next()).flatten()
        })
        .unwrap_or("UTF-8")
        .to_ascii_uppercase();
    match set.as_str() {
        "UTF-8" | "UTF8" => {
            let text = |b: &[u8], what: &str| String::from_utf8(b.to_vec()).map_err(|e| format!("{what}: not UTF-8: {e}"));
            Ok((text(aff, ".aff")?, text(dic, ".dic")?))
        }
        "ISO8859-1" | "ISO-8859-1" => Ok((decode_latin1(aff), decode_latin1(dic))),
        "ISO8859-2" | "ISO-8859-2" => Ok((decode_latin2(aff), decode_latin2(dic))),
        "KOI8-R" | "KOI8R" => Ok((decode_high(aff, KOI8_R_HIGH), decode_high(dic, KOI8_R_HIGH))),
        "CP1251" | "WINDOWS-1251" | "MICROSOFT-CP1251" => Ok((decode_high(aff, CP1251_HIGH), decode_high(dic, CP1251_HIGH))),
        other => Err(format!("unsupported encoding {other}")),
    }
}

/// The per-user Windows folders under `bases` (`%LOCALAPPDATA%`, then `%APPDATA%`): every app
/// folder first, then the shared `hunspell` ones, so a dictionary the user put in either app
/// folder wins over a shared one.
fn windows_user_dirs(bases: &[PathBuf], app_dir: Option<&str>) -> Vec<PathBuf> {
    let apps = app_dir.into_iter().flat_map(|app| bases.iter().map(move |b| b.join(app).join("dictionaries")));
    apps.chain(bases.iter().map(|b| b.join("hunspell"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_latin2_and_utf8() {
        let aff = b"SET ISO8859-2\nTRY a\n";
        let (a, d) = decode(aff, b"2\n\xbf\xf3\xb3w\n").unwrap();
        assert!(a.contains("TRY a"));
        assert_eq!(d, "2\nżółw\n");
        let (_, d) = decode("SET UTF-8\n".as_bytes(), "żółw".as_bytes()).unwrap();
        assert_eq!(d, "żółw");
        assert!(decode(b"SET SHIFT_JIS\n", b"").is_err());
        assert!(decode(b"SET UTF-8\n", b"\xff\xfe").is_err());
    }

    #[test]
    fn set_reads_only_the_charset_token() {
        // `ł` in ISO 8859-2 is 0xB3; a comment after the charset must not make it unsupported.
        let (_, d) = decode(b"SET ISO8859-2 # Polish\n", b"1\n\xb3\n").unwrap();
        assert_eq!(d, "1\nł\n");
        assert!(decode(b"SET UTF-8\tx\n", b"").is_ok());
        // Any whitespace separates SET from the charset, a tab included.
        let (_, d) = decode(b"SET\tISO8859-2\n", b"1\n\xb3\n").unwrap();
        assert_eq!(d, "1\nł\n");
        assert!(decode(b"SETTINGS ISO8859-2\n", b"\xb3").is_err(), "SET must be the whole directive");
        assert!(decode(b"SET \n", b"").is_ok(), "an empty SET is UTF-8");
        assert!(decode(b"SET SHIFT_JIS # no\n", b"").is_err(), "an unknown charset is still rejected");
    }

    #[test]
    fn decodes_latin1() {
        // `orçamento esperávamos` in ISO 8859-1: ç = 0xE7, á = 0xE1 (from wordcraft#2904).
        let (a, d) = decode(b"SET ISO8859-1\nTRY \xe7\n", b"1\nor\xe7amento/A\nesper\xe1vamos\n").unwrap();
        assert!(a.contains("TRY ç"), "{a}");
        assert_eq!(d, "1\norçamento/A\nesperávamos\n");
        assert!(decode(b"SET ISO-8859-1\n", b"\xe7").is_ok());
    }

    #[test]
    fn decodes_russian_single_byte_encodings() {
        // `ёлка` in KOI8-R (0xA3 0xCC 0xCB 0xC1) and in Windows-1251 (0xB8 0xEB 0xEA 0xE0).
        let (a, d) = decode(b"SET KOI8-R\nTRY \xcf\xc5\n", b"1\n\xa3\xcc\xcb\xc1/A\n").unwrap();
        assert!(a.contains("TRY ое"), "{a}");
        assert_eq!(d, "1\nёлка/A\n");
        let (_, d) = decode(b"SET microsoft-cp1251\n", b"1\n\xb8\xeb\xea\xe0 \xc4\xee\xec\n").unwrap();
        assert_eq!(d, "1\nёлка Дом\n");
        assert_eq!(decode_high(&[0x98], CP1251_HIGH), "\u{fffd}");
    }

    #[test]
    fn libreoffice_extension_dirs_follow_the_language() {
        assert_eq!(lo_ext_dirs("pl_PL"), ["dict-pl", "dict-pl-PL"]);
        assert_eq!(lo_ext_dirs("pt_BR"), ["dict-pt", "dict-pt-BR"]);
        assert_eq!(lo_ext_dirs("de-DE"), ["dict-de", "dict-de-DE"]);
        let search = Search::default();
        let dirs = search.dirs(Some(&lo_ext_dirs("pt_BR")));
        assert!(dirs.iter().any(|d| d.ends_with("dict-pt-BR")), "{dirs:?}");
        assert!(search.dirs(None).iter().all(|d| !d.to_string_lossy().contains("dict-")), "no language: no LibreOffice guess");
    }

    #[test]
    fn each_app_names_its_own_folder_and_variable() {
        let s = Search::for_app("GridCraft");
        assert_eq!(s.env_var.as_deref(), Some("GRIDCRAFT_DICTIONARIES"));
        assert_eq!(s.app_dir.as_deref(), Some("GridCraft"));
        let extra = std::env::temp_dir().join("craft-proof-extra");
        let s = Search { extra_dir: Some(extra.clone()), ..Search::for_app("PdfCraft") };
        assert_eq!(s.dirs(None).first(), Some(&extra), "the setting comes first");
    }

    /// The app's own folder under the user's application data is searched on every desktop
    /// system, before the shared system locations (storytold/wordcraft#1063).
    #[test]
    fn the_app_folder_comes_before_the_system_locations() {
        let dirs = Search::for_app("WordCraft").dirs(None);
        let at = |suffix: &str| dirs.iter().position(|d| d.ends_with(suffix));
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let (app, shared) = if cfg!(windows) {
            ("WordCraft/dictionaries", "hunspell")
        } else if cfg!(target_os = "macos") {
            ("Application Support/WordCraft/dictionaries", "/Library/Spelling")
        } else {
            ("wordcraft/dictionaries", "/usr/share/hunspell")
        };
        if std::env::var_os(if cfg!(windows) { "LOCALAPPDATA" } else { "HOME" }).is_none() {
            return; // no user folder to derive it from
        }
        let (Some(app), Some(shared)) = (at(app), at(shared)) else { panic!("{dirs:?}") };
        assert!(app < shared, "{dirs:?}");
    }

    #[test]
    fn on_windows_both_app_folders_come_before_both_shared_folders() {
        let bases = [PathBuf::from("L"), PathBuf::from("R")];
        let dirs = super::windows_user_dirs(&bases, Some("WordCraft"));
        let expected: Vec<PathBuf> =
            ["L/WordCraft/dictionaries", "R/WordCraft/dictionaries", "L/hunspell", "R/hunspell"].iter().map(PathBuf::from).collect();
        assert_eq!(dirs, expected);
        let last_app = dirs.iter().rposition(|d| d.ends_with("dictionaries"));
        let first_shared = dirs.iter().position(|d| d.ends_with("hunspell"));
        assert!(last_app < first_shared, "{dirs:?}");
        assert_eq!(super::windows_user_dirs(&bases, None), vec![PathBuf::from("L/hunspell"), PathBuf::from("R/hunspell")]);
    }

    #[test]
    fn finds_and_loads_a_pair_in_a_directory() {
        let dir = std::env::temp_dir().join(format!("craft-proof-hunspell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(find_in(&dir, &["xx_XX"]).is_none(), "no files: nothing found");
        std::fs::write(dir.join("xx_XX.aff"), "SET UTF-8\nSFX A Y 1\nSFX A 0 y .\n").unwrap();
        assert!(find_in(&dir, &["xx_XX"]).is_none(), "an .aff alone isn't a dictionary");
        std::fs::write(dir.join("xx_XX.dic"), "1\nkot/A\n").unwrap();
        let (aff, dic) = find_in(&dir, &["yy", "xx_XX"]).unwrap();
        let d = load(&aff, &dic).unwrap();
        assert!(d.check("kot") && d.check("koty") && !d.check("kotx"));
        assert!(load(&dir.join("missing.aff"), &dic).is_err());
        let found = Search { extra_dir: Some(dir.clone()), ..Search::default() }.find(&["xx_XX"]);
        assert_eq!(found, Some((aff, dic)), "the extra folder is searched");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
