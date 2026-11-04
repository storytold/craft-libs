//! The Turkish Hunspell dictionary (`FLAG num`: thousands of numeric suffix flags).
//!
//! Read by the shared affix engine ([`AffixDict`]), so Turkish follows Hunspell as every other
//! language does: homonyms on separate lines keep each one's flags, a `.dic` entry ends before
//! its morphological fields (`kitap/1 po:noun`), and an affix applies only where its condition
//! matches the stem (`SFX 1 0 lar [^e]` never makes `kalelar`).

use crate::affix::AffixDict;

/// The Turkish dictionary.
#[derive(Clone, Debug, Default)]
pub struct TrDict {
    dict: AffixDict,
}

impl crate::slot::Dictionary for TrDict {
    fn load(aff: &std::path::Path, dic: &std::path::Path) -> Result<Self, String> {
        super::load(aff, dic)
    }

    fn parse(aff: &str, dic: &str) -> Result<Self, String> {
        TrDict::from_hunspell(aff, dic)
    }

    fn is_empty(&self) -> bool {
        TrDict::is_empty(self)
    }
}

impl TrDict {
    /// Returns true if empty (no dictionary loaded).
    pub fn is_empty(&self) -> bool {
        self.dict.is_empty()
    }

    /// Load from `.aff` and `.dic` contents.
    pub fn from_hunspell(aff: &str, dic: &str) -> Result<Self, String> {
        Ok(TrDict { dict: AffixDict::from_hunspell(aff, dic, &[])? })
    }

    /// Is `word` exactly (case and all) a stem or an affixed form of one?
    pub fn contains(&self, word: &str) -> bool {
        self.dict.contains(word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CodeRabbit on craft-libs#484/#489: suffix conditions, homonyms and morphological fields,
    /// each answer as hunspell 1.7 gives it on the same files.
    #[test]
    fn conditions_homonyms_and_fields_follow_hunspell() {
        let aff = "SET UTF-8\nFLAG num\nSFX 1 Y 2\nSFX 1 0 lar [^e]\nSFX 1 0 ler e\nSFX 2 Y 1\nSFX 2 0 de .\n";
        let d = TrDict::from_hunspell(aff, "4\nkitap/1\nkale/1\nkale/2\nsonra po:adv\n").unwrap();
        for (w, right) in
            [("kitaplar", true), ("kaleler", true), ("kalelar", false), ("kitapler", false), ("kalede", true), ("sonra", true), ("kitap", true)]
        {
            assert_eq!(d.contains(w), right, "{w}");
        }
        let d = TrDict::from_hunspell(aff, "1\nkitap/1,2\tpo:noun\n").unwrap();
        assert!(d.contains("kitaplar") && d.contains("kitapde"), "both flags before the field");
    }

    /// #495 review: LibreOffice's `tr_TR.dic` has 920 entries with more than 1,024 flags
    /// (`abart`, `savaş`, `inan`: up to 3,926); such a stem and its forms are words.
    #[test]
    fn a_stem_with_thousands_of_flags_is_a_word() {
        let aff = "SET UTF-8\nFLAG num\nSFX 1 Y 1\nSFX 1 0 de .\nSFX 3999 Y 1\nSFX 3999 0 lar .\n";
        let flags = (1..=4000).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let d = TrDict::from_hunspell(aff, &format!("2\nabart/{flags}\nkitap\n")).unwrap();
        assert!(d.contains("abart") && d.contains("abartde") && d.contains("abartlar") && d.contains("kitap"));
        assert!(!d.contains("kitaplar"));
    }
}
