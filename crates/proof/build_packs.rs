//! Embed explicitly selected, previously downloaded Language packs (including licences).
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    path::{Path, PathBuf},
};

/// Generate embedded-pack source after validating every explicitly selected locale and file.
pub fn generate() -> Result<(String, Vec<String>, String), String> {
    println!("cargo::rerun-if-env-changed=CRAFT_PROOF_PACKS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_PROOF_EMBED_PACKS");
    let named = std::env::var("CRAFT_PROOF_EMBED_PACKS").unwrap_or_default();
    let names: BTreeSet<&str> = named.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let mut code = String::from("pub(crate) static PACKS: &[crate::packs::EmbeddedPack] = &[\n");
    let mut locales = Vec::new();
    let mut licences = String::new();
    let dir = std::env::var_os("CRAFT_PROOF_PACKS_DIR").map(PathBuf::from);
    if !names.is_empty() {
        validate_directory(dir.as_deref().ok_or("CRAFT_PROOF_EMBED_PACKS needs CRAFT_PROOF_PACKS_DIR")?)?;
    }
    for locale in names {
        validate_locale(locale)?;
        let base = dir.as_ref().ok_or("CRAFT_PROOF_EMBED_PACKS needs CRAFT_PROOF_PACKS_DIR")?.join(locale);
        // Track the folder too: a hyphenation-only pack can gain spelling files or notices.
        println!("cargo::rerun-if-changed={}", base.display());
        if !base.join("pack.toml").is_file() {
            return Err(format!("{}: no pack.toml", base.display()));
        }
        println!("cargo::rerun-if-changed={}", base.join("pack.toml").display());
        let metadata = std::fs::read_to_string(base.join("pack.toml")).map_err(|e| format!("{locale}: pack.toml: {e}"))?;
        validate_pack_locale(&metadata, locale)?;
        let aff = base.join("spelling.aff");
        let dic = base.join("spelling.dic");
        if aff.is_file() != dic.is_file() {
            return Err(format!("{locale}: spelling needs both aff and dic"));
        }
        let nonstandard = base.join("hyphenation.nonstandard");
        let patterns = base.join("hyphenation.pat");
        if nonstandard.is_file() && !patterns.is_file() {
            return Err(format!("{locale}: nonstandard patterns need the standard fallback"));
        }
        if !aff.is_file() && !patterns.is_file() {
            return Err(format!("{locale}: no language data"));
        }
        if patterns.is_file() {
            validate_hyphenation_locale(locale)?;
        }
        let aff = include(&aff, true)?;
        let dic = include(&dic, true)?;
        let patterns = include(&patterns, false)?;
        let nonstandard = include(&nonstandard, false)?;
        let _ = writeln!(code, "crate::packs::EmbeddedPack{{locale:{locale:?},aff:{aff},dic:{dic},patterns:{patterns},nonstandard:{nonstandard}}},");
        locales.push(locale.to_string());
        let mut files: Vec<_> = std::fs::read_dir(base.join("LICENCES"))
            .map_err(|e| format!("{locale}: licences: {e}"))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        if files.is_empty() {
            return Err(format!("{locale}: no licence texts"));
        }
        for file in files {
            println!("cargo::rerun-if-changed={}", file.display());
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let title = format!("Language pack {locale}: {}", file.file_name().unwrap_or_default().to_string_lossy());
            let _ = write!(licences, "({title:?}, {text:?}), ");
        }
    }
    code.push_str("];\n");
    Ok((code, locales, licences))
}
/// Emit an include for a verified pack file and its Cargo rebuild dependency.
fn include(path: &Path, bytes: bool) -> Result<String, String> {
    if !path.is_file() {
        return Ok(if bytes { "b\"\"" } else { "\"\"" }.into());
    }
    let path = path.to_str().ok_or("Language pack path is not UTF-8")?;
    println!("cargo::rerun-if-changed={path}");
    Ok(format!("{}!({path:?})", if bytes { "include_bytes" } else { "include_str" }))
}

// Accept only canonical tags that PackLocale::new can select without truncation.
/// Reject locale names whose filesystem or runtime selection semantics would change.
fn validate_locale(locale: &str) -> Result<(), String> {
    let mut parts = locale.split('-');
    let primary = parts.next().unwrap_or("");
    if locale.len() > 15
        || !(2..=8).contains(&primary.len())
        || !primary.bytes().all(|b| b.is_ascii_lowercase())
        || ["und", "zxx", "mul", "mis"].contains(&primary)
        || parts.any(|p| !(2..=8).contains(&p.len()) || !p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
    {
        return Err(format!("invalid embedded pack locale: {locale:?}"));
    }
    Ok(())
}

/// Reject aliases that cannot be reached by runtime hyphenation selection.
fn validate_hyphenation_locale(locale: &str) -> Result<(), String> {
    let primary = locale.split('-').next().unwrap_or_default();
    // These languages collapse to built-in ProofLang variants. Match the names
    // packs::hyphenation_locales actually requests, rather than accepting dead aliases.
    let reachable = match primary {
        "pl" | "pol" => locale == "pl",
        "bg" | "bul" => locale == "bg",
        "it" | "ita" => locale == "it",
        "hr" | "hrv" => locale == "hr",
        "tr" | "tur" => locale == "tr",
        "ru" | "rus" => locale == "ru",
        "cs" | "ces" | "cze" => locale == "cs",
        "id" | "ind" | "in" => locale == "id",
        "pt" | "por" => matches!(locale, "pt" | "pt-pt"),
        "de" | "deu" | "ger" => matches!(locale, "de" | "de-de" | "de-1996" | "de-ch"),
        "es" | "spa" => matches!(locale, "es" | "es-es" | "es-mx" | "es-419" | "es-uy"),
        "en" | "eng" => false, // English uses its bundled patterns.
        _ => true,
    };
    if reachable { Ok(()) } else { Err(format!("hyphenation locale {locale:?} cannot be selected by runtime lookup")) }
}

/// Reject pack paths that change meaning when included from Cargo OUT_DIR.
fn validate_directory(dir: &Path) -> Result<(), String> {
    if !dir.is_absolute() {
        return Err(format!("{} is not an absolute path", dir.display()));
    }
    if dir.to_str().is_none() {
        return Err(format!("{} is not valid UTF-8", dir.display()));
    }
    Ok(())
}
/// Require the selected locale and an Embeddable distribution class before including pack bytes.
fn validate_pack_locale(text: &str, folder: &str) -> Result<(), String> {
    let metadata: toml::Table = toml::from_str(text).map_err(|e| format!("pack.toml: {e}"))?;
    let locale = metadata.get("locale").and_then(toml::Value::as_str).ok_or("pack.toml: missing locale string")?;
    if locale != folder {
        return Err(format!("pack.toml names locale {locale:?}, not {folder:?}"));
    }
    // Download-only data may be installed separately, but must never enter a release binary.
    if metadata.get("class").and_then(toml::Value::as_str) != Some("Embeddable") {
        return Err("pack.toml: embedding requires class = \"Embeddable\"".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Regression: embedded patterns reject unreachable builtin aliases.
    #[test]
    fn embedded_patterns_reject_unreachable_builtin_aliases() {
        for tag in ["pl", "hu", "hu-hu", "pt-pt", "de-ch", "de-1996", "es-419", "sr-latn-rs"] {
            assert!(validate_hyphenation_locale(tag).is_ok(), "{tag}");
        }
        for tag in ["pl-pl", "pol", "pt-br", "de-li", "es-ar", "en-us", "ru-ru"] {
            assert!(validate_hyphenation_locale(tag).is_err(), "{tag}");
        }
    }
    /// Regression: embedded locale must survive runtime selection unchanged.
    #[test]
    fn embedded_locale_must_survive_runtime_selection_unchanged() {
        for tag in ["hu", "hu-hu", "sr-latn-rs", "abcdefgh-123456"] {
            assert!(validate_locale(tag).is_ok(), "{tag}");
        }
        for tag in ["abcdefghi", "hu-abcdefghi", "hu--hu", "hu-", "h", "hu-x-data", "hu-abcdefgh-abcd", "und", "HU", "12"] {
            assert!(validate_locale(tag).is_err(), "{tag}");
        }
    }
    /// Regression: embedded pack metadata must name its folder.
    #[test]
    fn embedded_pack_metadata_must_name_its_folder() {
        assert!(validate_pack_locale("locale = \"hu\"\nclass = \"Embeddable\"", "hu").is_ok());
        assert!(validate_pack_locale("locale = \"hu-hu\"\nclass = \"Embeddable\"", "hu").unwrap_err().contains("not \"hu\""));
        for text in ["[", "name = \"Hungarian\"", "locale = 42"] {
            assert!(validate_pack_locale(text, "hu").is_err());
        }
    }
    /// Only packs classified for redistribution may be embedded; missing/unknown classes fail closed.
    #[test]
    fn embedding_rejects_non_embeddable_classes() {
        for class in ["Download-only", "Excluded", "unknown", ""] {
            let text = format!("locale = \"hu\"\nclass = {class:?}\n");
            assert!(validate_pack_locale(&text, "hu").unwrap_err().contains("Embeddable"));
        }
        assert!(validate_pack_locale("locale = \"hu\"", "hu").is_err());
    }
    /// Regression: embedding rejects paths that change meaning inside out dir.
    #[test]
    fn embedding_rejects_paths_that_change_meaning_inside_out_dir() {
        assert!(validate_directory(Path::new("packs/hu")).unwrap_err().contains("absolute"));
        assert!(validate_directory(Path::new("")).is_err());
        assert!(validate_directory(&std::env::temp_dir()).is_ok());
    }
}
