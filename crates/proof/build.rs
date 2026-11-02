//! Embed the Language data from a craft-i18n checkout (storytold/craft-i18n, `language-data/`).
//!
//! `CRAFT_I18N_DIR`, an absolute path to a craft-i18n checkout: `$OUT_DIR/language_data.rs` holds
//! one constant per file in [`FILES`], each an `include_str!`/`include_bytes!` of
//! `$CRAFT_I18N_DIR/language-data/<path>`. Unset, or not a checkout, or a file missing (a warning):
//! that constant is empty, and the crate has no embedded Language data for it (English spelling
//! falls back to an installed Hunspell dictionary or the crate's own copy of the Moby word list, and
//! hyphenation that needs the data is off). With `CRAFT_I18N_REQUIRED=1` (releases) any of those
//! is an error instead.
//!
//! When every file in [`FILES`] this build embeds (English and the [`opted_in`] locales) is
//! there, the `craft_proof_data` cfg is set (the tests that pin
//! results of the data use it); when none is, `craft_proof_no_data` (the tests of a build without
//! the data). A partial data folder sets neither, so neither set of tests runs against it.
//!
//! `include_str!` takes a string, so a `CRAFT_I18N_DIR` that isn't valid UTF-8 can't be embedded:
//! it is treated like a folder without the data.
//!
//! English: SCOWL's en-US, en-GB (-ise) and en-CA dictionaries; en-AU is not embedded, as
//! Australian English is checked as British-spelling English (`ProofLang::EnGb`). The SCOWL
//! notice is embedded once (the four upstream READMEs differ only in their title and build line).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// (constant, path under `language-data/`, bytes rather than text). The first path part is the
/// locale: `en-*` and our own `*-extra.txt` supplements are always embedded, the others as
/// [`opted_in`] says.
const FILES: &[(&str, &str, bool)] = &[
    ("EN_US_DIC", "en-us/en-us.dic", true),
    ("EN_US_AFF", "en-us/spelling.aff", false),
    ("EN_US_SPELLING", "en-us/spelling.dic", false),
    ("EN_GB_AFF", "en-gb/spelling.aff", false),
    ("EN_GB_SPELLING", "en-gb/spelling.dic", false),
    ("EN_CA_AFF", "en-ca/spelling.aff", false),
    ("EN_CA_SPELLING", "en-ca/spelling.dic", false),
    ("SCOWL_NOTICE", "en-us/LICENCE-SCOWL.txt", false),
    ("EN_US_PAT", "en-us/hyphenation.pat", false),
    ("EXTRA_EN", "en-us/en-extra.txt", false),
    ("EN_VARIANTS", "en-us/en-variants.txt", false),
    ("PL_PAT", "pl/hyphenation.pat", false),
    ("PT_PAT", "pt/hyphenation.pat", false),
    ("IT_PAT", "it/hyphenation.pat", false),
    ("DE_PAT", "de/hyphenation.pat", false),
    ("EXTRA_RU", "ru/ru-extra.txt", false),
];

/// The locales embedded by default besides English: every carry-over locale, until Language
/// packs are published and pinned (CP4.4 then makes the default English only).
const DEFAULT_EMBED: &[&str] = &["de", "pl", "pt", "it", "ru"];

/// The locales this build embeds besides English: `CRAFT_PROOF_EMBED`, comma-separated lowercase
/// locales (`de,pl`), `all`, or `en` (English and our supplements only; the others come from
/// Language packs). Unset: [`DEFAULT_EMBED`].
fn opted_in() -> Vec<String> {
    let v = std::env::var("CRAFT_PROOF_EMBED").unwrap_or_default();
    let named: Vec<String> = v.split(',').map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).collect();
    if named.is_empty() { DEFAULT_EMBED.iter().map(|l| (*l).to_string()).collect() } else { named }
}

/// Our own word supplements (`*-extra.txt`): MIT, tiny, embedded in every build whatever the
/// locale; only upstream data waits for a Language build or a pack.
fn is_ours(rel: &str) -> bool {
    rel.ends_with("-extra.txt")
}

/// Is the file `rel` (of `locale`) embedded in this build?
fn embeds(rel: &str, locale: &str, opted: &[String]) -> bool {
    is_ours(rel) || locale.starts_with("en-") || opted.iter().any(|o| o == locale || o == "all")
}

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_REQUIRED");
    println!("cargo::rerun-if-env-changed=CRAFT_PROOF_EMBED");
    println!("cargo::rustc-check-cfg=cfg(craft_proof_data)");
    println!("cargo::rustc-check-cfg=cfg(craft_proof_no_data)");
    println!("cargo::rustc-check-cfg=cfg(craft_proof_embed, values(\"pl\", \"pt\", \"it\", \"de\", \"ru\"))");
    let opted = opted_in();
    let required = std::env::var("CRAFT_I18N_REQUIRED").is_ok_and(|v| v.trim() == "1");
    let level = if required { "error" } else { "warning" };
    let dir = match std::env::var_os("CRAFT_I18N_DIR").filter(|d| !d.is_empty()) {
        Some(d) => match language_data(&PathBuf::from(d)) {
            Ok(dir) => Some(dir),
            Err(e) => {
                println!("cargo::{level}=CRAFT_I18N_DIR: {e}; craft-proof embeds no Language data");
                None
            }
        },
        None => {
            if required {
                println!("cargo::error=CRAFT_I18N_REQUIRED=1 but CRAFT_I18N_DIR is unset: a release must embed the Language data");
            } else {
                println!(
                    "cargo::warning=CRAFT_I18N_DIR is unset: craft-proof embeds no Language data (English spelling falls back to system Hunspell or the built-in Moby word list; no hyphenation)"
                );
            }
            None
        }
    };
    let mut o = String::from("// Generated by craft-proof's build.rs from CRAFT_I18N_DIR/language-data. Do not edit.\n");
    // Files this build embeds when present (English and the opted-in locales), and those embedded.
    let (mut wanted_files, mut embedded) = (0, 0);
    let mut locales: Vec<String> = Vec::new();
    for &(name, rel, bytes) in FILES {
        let locale = rel.split('/').next().unwrap_or_default();
        let wanted = embeds(rel, locale, &opted);
        wanted_files += usize::from(wanted);
        let file = dir.as_ref().filter(|_| wanted).map(|d| d.join(rel)).filter(|f| f.is_file());
        if let (Some(d), None, true) = (&dir, &file, wanted) {
            println!("cargo::{level}=CRAFT_I18N_DIR: {} is missing; craft-proof embeds the rest without it", d.join(rel).display());
        }
        let (ty, empty, mac) = if bytes { ("&[u8]", "b\"\"", "include_bytes") } else { ("&str", "\"\"", "include_str") };
        // `language_data` only hands back a UTF-8 folder, so this is the path as it is.
        let value = match file.as_ref().and_then(|f| f.to_str()) {
            Some(f) => {
                println!("cargo::rerun-if-changed={f}");
                embedded += 1;
                if !locale.starts_with("en-") {
                    println!("cargo::rustc-cfg=craft_proof_embed=\"{locale}\"");
                }
                // The locales whose upstream data is embedded (our supplements don't count).
                if !is_ours(rel) && !locales.iter().any(|l| l == locale) {
                    locales.push(locale.to_string());
                }
                // `{:?}` escapes the path (Windows backslashes) as a Rust string literal.
                format!("{mac}!({f:?})")
            }
            None => empty.to_string(),
        };
        let _ = writeln!(o, "/// `language-data/{rel}` (empty when it isn't embedded).\npub(crate) static {name}: {ty} = {value};");
    }
    if embedded > 0 && embedded == wanted_files {
        println!("cargo::rustc-cfg=craft_proof_data");
    } else if embedded == 0 {
        println!("cargo::rustc-cfg=craft_proof_no_data");
    }
    let _ = writeln!(o, "/// The locales whose Language data is embedded.\npub(crate) static EMBEDDED_LOCALES: &[&str] = &{locales:?};");
    // The licence texts of the embedded locales (`language-data/<locale>/LICENCE*`), for About ›
    // Language data.
    let mut licences = String::new();
    if let Some(d) = &dir {
        for locale in &locales {
            let mut files: Vec<PathBuf> = std::fs::read_dir(d.join(locale))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_uppercase().starts_with("LICENCE")))
                .collect();
            files.sort();
            for f in files {
                println!("cargo::rerun-if-changed={}", f.display());
                let title = format!("{locale}: {}", f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                let _ = write!(licences, "({title:?}, include_str!({:?})), ", f.display().to_string());
            }
        }
    }
    let _ = writeln!(
        o,
        "/// (title, text) of each embedded locale's licence files.\npub(crate) static EMBEDDED_LICENCES: &[(&str, &str)] = &[{licences}];"
    );
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("language_data.rs");
    if let Err(e) = std::fs::write(&out, o) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// `dir/language-data`, when `dir` is an absolute path to a checkout that has one.
fn language_data(dir: &Path) -> Result<PathBuf, String> {
    // Build scripts run in the crate's directory and `include_str!` resolves against the generated
    // file, so a relative path would mean two different places. Not canonicalized: on Windows that
    // gives `\\?\C:\…`, which `include_str!` can't read.
    if !dir.is_absolute() {
        return Err(format!("{} is not an absolute path", dir.display()));
    }
    if dir.to_str().is_none() {
        return Err(format!("{} is not valid UTF-8, which include_str! needs", dir.display()));
    }
    let data = dir.join("language-data");
    if !data.is_dir() {
        // Notice when the checkout gains its Language data.
        println!("cargo::rerun-if-changed={}", dir.display());
        return Err(format!("{} has no language-data/ (a craft-i18n checkout older than its Language data?)", dir.display()));
    }
    println!("cargo::rerun-if-changed={}", data.display());
    Ok(data)
}
