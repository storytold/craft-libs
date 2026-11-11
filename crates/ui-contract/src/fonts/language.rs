//! The scripts a language is written in.

use craft_i18n::Han;

/// The scripts a language is written in, by language tag. [`I18nScripts`] answers from
/// craft-i18n; an app with its own table implements this instead.
pub trait LanguageScripts {
    /// ISO 15924 codes of the language's interface text, main script first: `ja` → `Hani`, `Hira`,
    /// `Kana`; `ar` → `Arab`. The unions `Jpan`, `Kore`, `Hans` and `Hant` are accepted too.
    fn scripts(&self, language: &str) -> Vec<&'static str>;

    /// The regional Han forms the language uses, `None` for a language not written with Han.
    fn han(&self, language: &str) -> Option<Han>;
}

/// [`LanguageScripts`] from craft-i18n's script table ([`craft_i18n::scripts`],
/// [`craft_i18n::han`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct I18nScripts;

impl LanguageScripts for I18nScripts {
    fn scripts(&self, language: &str) -> Vec<&'static str> {
        craft_i18n::scripts(language).iter().map(|s| s.code()).collect()
    }

    fn han(&self, language: &str) -> Option<Han> {
        craft_i18n::han(language)
    }
}
