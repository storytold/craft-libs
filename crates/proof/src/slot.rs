//! The installed Hunspell dictionary of a proofing language, in one shape for every language:
//! found on first use, replaced with [`DictSlot::install`] and forgotten with [`DictSlot::reload`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::affix::AffixDict;

/// A dictionary a language reads: the affix engine's ([`AffixDict`]) or Turkish's own.
pub(crate) trait Dictionary: Default + Send + Sync {
    /// The installed `.aff` and `.dic` files, read.
    fn load(aff: &Path, dic: &Path) -> Result<Self, String>;
    /// Hunspell `.aff` and `.dic` text (decoded to UTF-8), parsed.
    fn parse(aff: &str, dic: &str) -> Result<Self, String>;
    /// No words: the language has no spelling marks.
    fn is_empty(&self) -> bool;
}

impl Dictionary for AffixDict {
    fn load(aff: &Path, dic: &Path) -> Result<Self, String> {
        crate::hunspell::load(aff, dic)
    }

    fn parse(aff: &str, dic: &str) -> Result<Self, String> {
        AffixDict::from_hunspell(aff, dic, &[])
    }

    fn is_empty(&self) -> bool {
        AffixDict::is_empty(self)
    }
}

/// What every language's slot answers, whatever its dictionary type (the language table in
/// lib.rs holds them side by side).
pub(crate) trait Slot: Sync {
    /// See [`DictSlot::name`].
    fn name(&self) -> &'static str;
    /// See [`DictSlot::installed_path`].
    fn installed_path(&self) -> Option<PathBuf>;
    /// See [`DictSlot::has_dictionary`].
    fn has_dictionary(&self) -> bool;
    /// See [`DictSlot::reload`].
    fn reload(&self);
    /// See [`DictSlot::names`].
    #[cfg(test)]
    fn names(&self) -> &'static [&'static str];
}

impl<D: Dictionary> Slot for DictSlot<D> {
    fn name(&self) -> &'static str {
        DictSlot::name(self)
    }

    fn installed_path(&self) -> Option<PathBuf> {
        DictSlot::installed_path(self)
    }

    fn has_dictionary(&self) -> bool {
        DictSlot::has_dictionary(self)
    }

    fn reload(&self) {
        DictSlot::reload(self);
    }

    #[cfg(test)]
    fn names(&self) -> &'static [&'static str] {
        DictSlot::names(self)
    }
}

/// The dictionary in use and whether it came from [`DictSlot::install`] rather than a file on
/// disk (it then has no file to report, even if a matching file also happens to exist): one lock,
/// so a reload and an install can't interleave their two fields.
struct State<D> {
    dict: Option<Arc<D>>,
    in_memory: bool,
    /// Counts reloads, so a load that started before one is not kept.
    reloads: u64,
}

/// One language's dictionary.
pub(crate) struct DictSlot<D> {
    /// File names the dictionary may have, the usual one first (also the key of its load error).
    names: &'static [&'static str],
    /// The language's English name, for the log.
    english: &'static str,
    state: RwLock<State<D>>,
}

impl<D: Dictionary> DictSlot<D> {
    pub(crate) const fn new(names: &'static [&'static str], english: &'static str) -> Self {
        DictSlot { names, english, state: RwLock::new(State { dict: None, in_memory: false, reloads: 0 }) }
    }

    fn read(&self) -> RwLockReadGuard<'_, State<D>> {
        self.state.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, State<D>> {
        self.state.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The Hunspell name the dictionary is known by (`pl_PL`): the first of its file names.
    pub(crate) fn name(&self) -> &'static str {
        self.names.first().copied().unwrap_or_default()
    }

    /// The file names the dictionary may have, the usual one first.
    #[cfg(test)]
    pub(crate) fn names(&self) -> &'static [&'static str] {
        self.names
    }

    /// The dictionary: the installed one (read on first use), one given with [`Self::install`], or
    /// an empty one when none is available (the language's spelling is off then).
    pub(crate) fn get(&self) -> Arc<D> {
        let reloads = {
            let s = self.read();
            if let Some(d) = s.dict.as_ref() {
                return d.clone();
            }
            s.reloads
        };
        self.keep(reloads, Arc::new(self.load_installed()))
    }

    /// Keep a dictionary loaded after `reloads` reloads. If the folder was reloaded since, the
    /// load may have read the old folder: it serves this one check and the next one loads again.
    fn keep(&self, reloads: u64, loaded: Arc<D>) -> Arc<D> {
        let mut s = self.write();
        if s.reloads != reloads {
            return s.dict.clone().unwrap_or(loaded);
        }
        // Another thread (or an install) may have filled it meanwhile: that one wins.
        s.dict.get_or_insert(loaded).clone()
    }

    fn load_installed(&self) -> D {
        self.load_from(crate::hunspell::find(self.names), crate::data::PACKS)
    }

    /// Prefer the installed dictionary, then embedded data if the installed pair is absent or unreadable.
    fn load_from(&self, installed: Option<(PathBuf, PathBuf)>, table: &[crate::packs::EmbeddedPack]) -> D {
        let name = self.name();
        let Some((aff, dic)) = installed else {
            if let Some(dictionary) = self.load_embedded(table) {
                return dictionary;
            }
            log::info!("{} spelling: no Hunspell {name} dictionary available", self.english);
            return D::default();
        };
        let loaded = D::load(&aff, &dic)
            .and_then(|dictionary| if dictionary.is_empty() { Err("dictionary contains no words".into()) } else { Ok(dictionary) });
        if let Err(error) = &loaded {
            log::warn!("{} spelling: {error}", self.english);
            let fallback = self.load_embedded(table);
            // `installed_dictionaries` still names this file, even when embedded data works.
            // Preserve its failure so `dictionary_load_error` explains which source failed.
            let _ = crate::hunspell::note_load::<()>(name, &dic, Err(error.clone()));
            return fallback.unwrap_or_default();
        }
        crate::hunspell::note_load(name, &dic, loaded).unwrap_or_default()
    }

    /// Try the selected embedded packs with the same aliases and parser as an installed dictionary.
    fn load_embedded(&self, table: &[crate::packs::EmbeddedPack]) -> Option<D> {
        let packs = crate::packs::embedded_spelling_in(table, self.names);
        if packs.is_empty() {
            crate::hunspell::clear_load_error(self.name());
        }
        for pack in packs {
            let read = crate::hunspell::decode(pack.aff, pack.dic)
                .and_then(|(aff, dic)| D::parse(&aff, &dic))
                .and_then(|dictionary| if dictionary.is_empty() { Err("dictionary contains no words".into()) } else { Ok(dictionary) });
            let source = PathBuf::from(format!("<embedded Language pack {}>", pack.locale));
            match crate::hunspell::note_load(self.name(), &source, read) {
                Ok(dictionary) => return Some(dictionary),
                Err(error) => log::warn!("{} spelling: {error}", self.english),
            }
        }
        None
    }

    /// Use this dictionary from now on (Hunspell `.aff` and `.dic` text, decoded to UTF-8), for
    /// example one the user picked. On error the current dictionary stays.
    pub(crate) fn install(&self, aff: &str, dic: &str) -> Result<(), String> {
        let d = D::parse(aff, dic)?;
        let mut slot = self.write();
        slot.dict = Some(Arc::new(d));
        slot.in_memory = true;
        Ok(())
    }

    /// Forget the dictionary, so the next check looks for an installed one again (after
    /// [`crate::hunspell::set_dictionary_dir`] or a dictionary was installed).
    pub(crate) fn reload(&self) {
        let mut slot = self.write();
        slot.dict = None;
        slot.in_memory = false;
        slot.reloads = slot.reloads.wrapping_add(1);
    }

    /// The `.dic` file of the installed dictionary, searching the dictionaries folder and the
    /// system locations now (File › Options › Proofing shows it); `None` when there is none, or
    /// when the dictionary in use was given with [`Self::install`] and so has no file.
    pub(crate) fn installed_path(&self) -> Option<PathBuf> {
        if self.read().in_memory {
            return None;
        }
        crate::hunspell::find(self.names).map(|(_, dic)| dic)
    }

    /// Is a dictionary available (spelling is checked)?
    pub(crate) fn has_dictionary(&self) -> bool {
        !self.get().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SLOT: DictSlot<AffixDict> = DictSlot::new(&["zz_SLOT_TEST"], "Test");

    #[test]
    fn an_installed_dictionary_has_no_file_until_reloaded() {
        assert!(!SLOT.has_dictionary(), "nothing installed under this name");
        assert!(SLOT.install("FLAG bogus\n", "1\nkot\n").is_err(), "unsupported files are refused");
        assert!(!SLOT.has_dictionary(), "a refused install keeps the current one");
        SLOT.install("", "1\nkot\n").unwrap();
        assert!(SLOT.has_dictionary() && SLOT.get().check("kot"));
        assert_eq!(SLOT.installed_path(), None);
        SLOT.reload();
        assert!(!SLOT.has_dictionary(), "reloaded from disk, where there is none");
        assert_eq!(SLOT.name(), "zz_SLOT_TEST");
    }

    /// Built-in slots use matching embedded dictionaries, including regional fallback and load diagnostics.
    #[test]
    fn builtin_slots_load_embedded_spelling_with_alias_fallback() {
        crate::hold_dictionary_tests();
        for (names, regional, language) in [(&["pl_PL", "pl-PL", "pl"][..], "pl-pl", "pl"), (&["pt_PT", "pt-PT", "pt"][..], "pt-pt", "pt")] {
            let packs = [
                crate::packs::EmbeddedPack { locale: regional, aff: b"SET UTF-8\n\xff", dic: b"1\nword\n", patterns: "", nonstandard: "" },
                crate::packs::EmbeddedPack { locale: language, aff: b"SET UTF-8\n", dic: b"1\nword\n", patterns: "", nonstandard: "" },
            ];
            let slot: DictSlot<AffixDict> = DictSlot::new(names, "Test");
            let dictionary = slot.load_embedded(&packs).unwrap();
            assert!(dictionary.check("word") && !dictionary.check("wrong"));
            assert!(crate::hunspell::load_error(slot.name()).is_none(), "successful fallback clears the failed regional diagnostic");
            let _ = slot.keep(0, Arc::new(dictionary));
            assert!(slot.has_dictionary(), "the loaded slot must not report a missing dictionary");
            assert!(slot.load_embedded(&[]).is_none());
        }
    }

    /// A regional spelling-only pack is a fallback, but another language or script never is.
    #[test]
    fn builtin_embedded_spelling_uses_regional_fallback_without_crossing_scripts() {
        crate::hold_dictionary_tests();
        for (names, regional, wrong_script) in [
            (&["pl_PL", "pl-PL", "pl"][..], "pl-cz", "pl-cyrl"),
            (&["sr_Cyrl_RS", "sr_Cyrl", "sr"][..], "sr-cyrl-ba", "sr-latn-rs"),
            (&["sr_Latn_RS", "sr_Latn"][..], "sr-latn-ba", "sr-cyrl-rs"),
        ] {
            let packs = [
                crate::packs::EmbeddedPack { locale: wrong_script, aff: b"SET UTF-8\n", dic: b"1\nwrongscript\n", patterns: "", nonstandard: "" },
                crate::packs::EmbeddedPack { locale: "de", aff: b"SET UTF-8\n", dic: b"1\nunrelated\n", patterns: "", nonstandard: "" },
                crate::packs::EmbeddedPack { locale: regional, aff: b"SET UTF-8\n", dic: b"1\nregionalword\n", patterns: "", nonstandard: "" },
            ];
            let slot: DictSlot<AffixDict> = DictSlot::new(names, "Test");
            assert!(slot.load_embedded(&packs[..2]).is_none(), "unrelated language/script is not a fallback");
            let dictionary = slot.load_embedded(&packs).unwrap();
            assert!(dictionary.check("regionalword") && !dictionary.check("wrongscript") && !dictionary.check("unrelated"));
            let _ = slot.keep(0, Arc::new(dictionary));
            assert!(slot.has_dictionary());
        }
    }

    /// An empty affix file is valid; only a missing word list makes an embedded resource unusable.
    #[test]
    fn builtin_embedded_spelling_accepts_empty_affixes_but_not_empty_word_lists() {
        crate::hold_dictionary_tests();
        let slot: DictSlot<AffixDict> = DictSlot::new(&["pl_PL", "pl"], "Test");
        let packs = [
            crate::packs::EmbeddedPack { locale: "pl-pl", aff: b"SET UTF-8\n", dic: b"", patterns: "a1b", nonstandard: "" },
            crate::packs::EmbeddedPack { locale: "pl", aff: b"", dic: b"1\nplainword\n", patterns: "", nonstandard: "" },
        ];
        assert!(slot.load_embedded(&packs[..1]).is_none());
        let dictionary = slot.load_embedded(&packs).unwrap();
        assert!(dictionary.check("plainword") && !dictionary.check("plainwrong"));
        let _ = slot.keep(0, Arc::new(dictionary));
        assert!(slot.has_dictionary());
        assert!(crate::hunspell::load_error(slot.name()).is_none());
    }

    /// A parsed-empty exact pack cannot hide a usable regional/language fallback.
    #[test]
    fn builtin_parsed_empty_dictionary_tries_the_next_embedded_pack() {
        crate::hold_dictionary_tests();
        let slot: DictSlot<AffixDict> = DictSlot::new(&["pl_PL", "pl"], "Test");
        let packs = [
            crate::packs::EmbeddedPack { locale: "pl-pl", aff: b"SET UTF-8\n", dic: b"0\n", patterns: "", nonstandard: "" },
            crate::packs::EmbeddedPack { locale: "pl", aff: b"", dic: b"1\nfallbackword\n", patterns: "", nonstandard: "" },
        ];
        assert!(slot.load_embedded(&packs[..1]).is_none());
        assert!(crate::hunspell::load_error(slot.name()).unwrap().1.contains("no words"));
        let dictionary = slot.load_embedded(&packs).unwrap();
        assert!(dictionary.check("fallbackword") && !dictionary.check("wrongword"));
        assert!(crate::hunspell::load_error(slot.name()).is_none());
        let _ = slot.keep(0, Arc::new(dictionary));
        assert!(slot.has_dictionary());
    }

    /// Missing or invalid native files use embedded data, while valid native files retain precedence.
    #[test]
    fn builtin_failed_native_load_tries_embedded_and_preserves_failure_diagnostics() {
        crate::hold_dictionary_tests();
        let root = tempfile::Builder::new().prefix("craft-proof-native-embedded-").tempdir().unwrap();
        let (aff, dic) = (root.path().join("pl_PL.aff"), root.path().join("pl_PL.dic"));
        std::fs::write(&dic, "1\nnativeword\n").unwrap();
        let packs = [crate::packs::EmbeddedPack { locale: "pl", aff: b"", dic: b"1\nembeddedword\n", patterns: "", nonstandard: "" }];
        let slot: DictSlot<AffixDict> = DictSlot::new(&["pl_PL", "pl"], "Test");
        for invalid in [None, Some("FLAG invalid\n")] {
            if let Some(text) = invalid {
                std::fs::write(&aff, text).unwrap();
            }
            let loaded = slot.load_from(Some((aff.clone(), dic.clone())), &packs);
            assert!(loaded.check("embeddedword") && !loaded.check("nativeword"));
            let (source, reason) = crate::hunspell::load_error(slot.name()).unwrap();
            assert_eq!(source, dic);
            assert!(!reason.is_empty());
            assert_eq!(crate::dictionary_load_error(crate::ProofLang::Pl), Some(reason));
        }
        assert!(slot.load_from(Some((aff.clone(), dic.clone())), &[]).is_empty());
        let (source, error) = crate::hunspell::load_error(slot.name()).unwrap();
        assert_eq!(source, dic);
        assert!(!error.is_empty());
        std::fs::write(&aff, "SET UTF-8\n").unwrap();
        let loaded = slot.load_from(Some((aff, dic)), &packs);
        assert!(loaded.check("nativeword") && !loaded.check("embeddedword"));
        assert!(crate::hunspell::load_error(slot.name()).is_none());
    }

    /// A parsed-empty native dictionary cannot disable a usable embedded dictionary.
    #[test]
    fn builtin_empty_native_dictionary_uses_embedded_and_reports_original_source() {
        crate::hold_dictionary_tests();
        let root = tempfile::Builder::new().prefix("craft-proof-empty-native-").tempdir().unwrap();
        let (aff, dic) = (root.path().join("pl_PL.aff"), root.path().join("pl_PL.dic"));
        std::fs::write(&aff, "SET UTF-8\n").unwrap();
        std::fs::write(&dic, "0\n").unwrap();
        let slot: DictSlot<AffixDict> = DictSlot::new(&["pl_PL", "pl"], "Test");
        let packs = [crate::packs::EmbeddedPack { locale: "pl", aff: b"", dic: b"1\nfallbackword\n", patterns: "", nonstandard: "" }];
        let dictionary = slot.load_from(Some((aff, dic.clone())), &packs);
        assert!(dictionary.check("fallbackword") && !dictionary.check("wrongword"));
        let (file, reason) = crate::hunspell::load_error(slot.name()).unwrap();
        assert_eq!(file, dic);
        assert!(reason.contains("no words"));
        assert_eq!(crate::dictionary_load_error(crate::ProofLang::Pl), Some(reason));
        let _ = slot.keep(0, Arc::new(dictionary));
        assert!(slot.has_dictionary());
        crate::hunspell::clear_load_error(slot.name());
    }

    /// A load that started before a dictionaries-folder change is not kept after its reload.
    #[test]
    fn a_load_from_before_a_reload_is_not_kept() {
        let slot: DictSlot<AffixDict> = DictSlot::new(&["zz_SLOT_RELOAD_TEST"], "Test");
        let before = slot.read().reloads;
        slot.reload();
        let _ = slot.keep(before, Arc::new(AffixDict::default()));
        assert!(slot.read().dict.is_none(), "the next check loads from the new folder");
        let now = slot.read().reloads;
        let _ = slot.keep(now, Arc::new(AffixDict::default()));
        assert!(slot.read().dict.is_some(), "a load since the reload is kept");
    }
}
