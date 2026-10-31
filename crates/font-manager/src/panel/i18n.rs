//! Interface strings through craft-i18n: the English text is the lookup key.

use std::cell::Cell;
use std::sync::LazyLock;

use craft_i18n::{Embedded, FileSize, I18n, Lang};

/// The embedded translation catalogs: the panel's own Japanese and Arabic rows, until its rows
/// are part of craft-i18n.
static CATALOGS: &Embedded = &[("ja", "", include_str!("catalogs/ja.tsv")), ("ar", "", include_str!("catalogs/ar.tsv"))];

/// The interface languages offered, in menu order.
pub const LANGUAGES: &[&str] = &["en", "ja", "ar"];

pub static I18N: LazyLock<I18n> = LazyLock::new(|| {
    let (i18n, problems) = I18n::new(CATALOGS, LANGUAGES);
    for problem in problems {
        log::warn!("translation catalog: {problem}; that string shows in English");
    }
    i18n
});

thread_local! {
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// The interface language.
pub fn current() -> Lang {
    CURRENT.get()
}

pub fn set_current(lang: Lang) {
    CURRENT.set(lang);
}

/// Translates a literal.
macro_rules! tl {
    ($s:expr) => {
        $crate::panel::i18n::I18N.t($crate::panel::i18n::current(), $s)
    };
}
pub(crate) use tl;

/// Translates `template` and fills its `{name}` placeholders.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    craft_i18n::fmt(I18N.t(current(), template), args)
}

/// A count with its noun, `{n}` in the English forms.
pub fn tn(n: u64, one: &str, other: &str) -> String {
    I18N.trn(current(), n, one, other)
}

/// A download size, `1.5 MB`.
pub fn size(bytes: u64) -> String {
    I18N.file_size(current(), bytes, FileSize::DECIMAL)
}

/// A count on its own, in the region's digits and grouping.
pub fn number(n: u64) -> String {
    I18N.format().integer(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row reads. The strings the panel shows without a row are listed (run with
    /// `--nocapture`); they show in English, so a missing row never fails.
    #[test]
    fn catalogs_load_and_missing_rows_are_listed() {
        let (_, problems) = I18n::new(CATALOGS, LANGUAGES);
        assert!(problems.is_empty(), "{problems:#?}");
        // English-only by design: style and weight names, licence and product names.
        let english = [
            "Thin",
            "ExtraLight",
            "Light",
            "Regular",
            "Medium",
            "SemiBold",
            "Bold",
            "ExtraBold",
            "Black",
            "Italic",
            "Variable {from}–{to}",
            "SIL Open Font License",
            "Apache License 2.0",
            "Ubuntu Font Licence",
            "Google Fonts",
            "{source}: {error}",
        ];
        let sources =
            [include_str!("app.rs"), include_str!("browse.rs"), include_str!("details.rs"), include_str!("sources.rs"), include_str!("available.rs")];
        let mut keys = Vec::new();
        for source in sources {
            for marker in ["tl!(", "fmt(", "in_os_fonts("] {
                for part in source.split(marker).skip(1) {
                    if let Some(key) = part.trim_start().strip_prefix('"').and_then(|p| p.find('"').map(|end| &p[..end])) {
                        keys.push(key.to_owned());
                    }
                }
            }
        }
        keys.retain(|k| !english.contains(&k.as_str()));
        assert!(keys.len() > 60, "{}", keys.len());
        for lang in ["ja", "ar"] {
            let lang = I18N.match_tag(lang).unwrap_or(Lang::EN);
            let missing: Vec<_> = keys.iter().filter(|k| I18N.t(lang, k) == k.as_str()).collect();
            if !missing.is_empty() {
                println!("{} is missing {} strings: {missing:#?}", I18N.code(lang), missing.len());
            }
        }
    }
}
