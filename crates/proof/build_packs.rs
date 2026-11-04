//! Embed explicitly selected, previously downloaded Language packs (including licences).
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    path::{Path, PathBuf},
};

pub fn generate() -> Result<(String, Vec<String>, String), String> {
    println!("cargo::rerun-if-env-changed=CRAFT_PROOF_PACKS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_PROOF_EMBED_PACKS");
    let named = std::env::var("CRAFT_PROOF_EMBED_PACKS").unwrap_or_default();
    let names: BTreeSet<&str> = named.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let mut code = String::from("pub(crate) static PACKS: &[crate::packs::EmbeddedPack] = &[\n");
    let mut locales = Vec::new();
    let mut licences = String::new();
    let dir = std::env::var_os("CRAFT_PROOF_PACKS_DIR").map(PathBuf::from);
    for locale in names {
        if locale.len() > 35
            || locale.starts_with('-')
            || locale.ends_with('-')
            || !locale.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!("invalid embedded pack locale: {locale:?}"));
        }
        let base = dir.as_ref().ok_or("CRAFT_PROOF_EMBED_PACKS needs CRAFT_PROOF_PACKS_DIR")?.join(locale);
        if !base.join("pack.toml").is_file() {
            return Err(format!("{}: no pack.toml", base.display()));
        }
        println!("cargo::rerun-if-changed={}", base.join("pack.toml").display());
        let aff = base.join("spelling.aff");
        let dic = base.join("spelling.dic");
        if aff.is_file() != dic.is_file() {
            return Err(format!("{locale}: spelling needs both aff and dic"));
        }
        let nonstandard = base.join("hyphenation.nonstandard");
        let patterns = if nonstandard.is_file() { nonstandard } else { base.join("hyphenation.pat") };
        if !aff.is_file() && !patterns.is_file() {
            return Err(format!("{locale}: no language data"));
        }
        let aff = include(&aff, true)?;
        let dic = include(&dic, true)?;
        let patterns = include(&patterns, false)?;
        let _ = writeln!(code, "crate::packs::EmbeddedPack{{locale:{locale:?},aff:{aff},dic:{dic},patterns:{patterns}}},");
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
fn include(path: &Path, bytes: bool) -> Result<String, String> {
    if !path.is_file() {
        return Ok(if bytes { "b\"\"" } else { "\"\"" }.into());
    }
    let path = path.to_str().ok_or("Language pack path is not UTF-8")?;
    println!("cargo::rerun-if-changed={path}");
    Ok(format!("{}!({path:?})", if bytes { "include_bytes" } else { "include_str" }))
}
