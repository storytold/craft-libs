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
        let (loaded, diagnostic) = self.load_installed();
        self.keep(reloads, Arc::new(loaded), || crate::hunspell::publish_load_diagnostic(self.name(), diagnostic))
    }

    /// Keep a dictionary loaded after `reloads` reloads. If the folder was reloaded since, the
    /// load may have read the old folder: it serves this one check and the next one loads again.
    fn keep(&self, reloads: u64, loaded: Arc<D>, note: impl FnOnce()) -> Arc<D> {
        let mut s = self.write();
        if s.reloads != reloads {
            return s.dict.clone().unwrap_or(loaded);
        }
        // Another thread (or an install) may have filled it meanwhile: that one wins,
        // including its diagnostic.
        if let Some(found) = &s.dict {
            return found.clone();
        }
        note();
        s.dict.get_or_insert(loaded).clone()
    }

    fn load_installed(&self) -> (D, crate::hunspell::LoadDiagnostic) {
        self.load_from(crate::hunspell::source_candidates(self.names).into_iter().next(), crate::data::PACKS)
    }

    /// Prefer the selected source, retaining its diagnostic if embedded fallback is needed.
    fn load_from(
        &self,
        installed: Option<crate::hunspell::DictionarySource>,
        table: &[crate::packs::EmbeddedPack],
    ) -> (D, crate::hunspell::LoadDiagnostic) {
        let Some(source) = installed else {
            let (dictionary, diagnostic) = self.load_embedded(table);
            return (dictionary.unwrap_or_default(), diagnostic);
        };
        let loaded = match &source {
            crate::hunspell::DictionarySource::Files { aff, dic } => D::load(aff, dic),
            _ => source.decoded().and_then(|(aff, dic)| D::parse(&aff, &dic)),
        }
        .and_then(|d| if d.is_empty() { Err("dictionary contains no words".into()) } else { Ok(d) });
        let diagnostic = Some((source, loaded.as_ref().map(|_| ()).map_err(Clone::clone)));
        match loaded {
            Ok(dictionary) => (dictionary, diagnostic),
            Err(error) => {
                log::warn!("{} spelling: {error}", self.english);
                (self.load_embedded(table).0.unwrap_or_default(), diagnostic)
            }
        }
    }

    /// Parse static fallback bytes without publishing a diagnostic before the cache accepts them.
    fn load_embedded(&self, table: &[crate::packs::EmbeddedPack]) -> (Option<D>, crate::hunspell::LoadDiagnostic) {
        let mut diagnostic = None;
        for pack in crate::packs::embedded_spelling_in(table, self.names) {
            let source = crate::hunspell::DictionarySource::embedded(pack);
            let read = source
                .decoded()
                .and_then(|(aff, dic)| D::parse(&aff, &dic))
                .and_then(|d| if d.is_empty() { Err("dictionary contains no words".into()) } else { Ok(d) });
            diagnostic = Some((source, read.as_ref().map(|_| ()).map_err(Clone::clone)));
            match read {
                Ok(dictionary) => return (Some(dictionary), diagnostic),
                Err(error) => log::warn!("{} spelling: {error}", self.english),
            }
        }
        (None, diagnostic)
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
        crate::hunspell::source_candidates(self.names).into_iter().next().and_then(|s| s.file())
    }

    /// Is a dictionary available (spelling is checked)?
    pub(crate) fn has_dictionary(&self) -> bool {
        !self.get().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exercise the same loader directly; production publishes only in DictSlot::keep.
    fn load_embedded<D: Dictionary>(slot: &DictSlot<D>, table: &[crate::packs::EmbeddedPack]) -> Option<D> {
        let (dictionary, diagnostic) = slot.load_embedded(table);
        crate::hunspell::publish_load_diagnostic(slot.name(), diagnostic);
        dictionary
    }

    fn load_from<D: Dictionary>(slot: &DictSlot<D>, installed: Option<(PathBuf, PathBuf)>, table: &[crate::packs::EmbeddedPack]) -> D {
        let source = installed.map(|(aff, dic)| crate::hunspell::DictionarySource::Files { aff, dic });
        let (dictionary, diagnostic) = slot.load_from(source, table);
        crate::hunspell::publish_load_diagnostic(slot.name(), diagnostic);
        dictionary
    }

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
            let dictionary = load_embedded(&slot, &packs).unwrap();
            assert!(dictionary.check("word") && !dictionary.check("wrong"));
            assert!(crate::hunspell::source_load_error(slot.name()).is_none(), "successful fallback clears the failed regional diagnostic");
            let _ = slot.keep(0, Arc::new(dictionary), || {});
            assert!(slot.has_dictionary(), "the loaded slot must not report a missing dictionary");
            assert!(load_embedded(&slot, &[]).is_none());
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
            assert!(load_embedded(&slot, &packs[..2]).is_none(), "unrelated language/script is not a fallback");
            let dictionary = load_embedded(&slot, &packs).unwrap();
            assert!(dictionary.check("regionalword") && !dictionary.check("wrongscript") && !dictionary.check("unrelated"));
            let _ = slot.keep(0, Arc::new(dictionary), || {});
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
        assert!(load_embedded(&slot, &packs[..1]).is_none());
        let dictionary = load_embedded(&slot, &packs).unwrap();
        assert!(dictionary.check("plainword") && !dictionary.check("plainwrong"));
        let _ = slot.keep(0, Arc::new(dictionary), || {});
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
        assert!(load_embedded(&slot, &packs[..1]).is_none());
        assert!(crate::hunspell::source_load_error(slot.name()).unwrap().contains("no words"));
        assert!(crate::hunspell::load_error(slot.name()).is_none(), "embedded diagnostics never invent a file path");
        let dictionary = load_embedded(&slot, &packs).unwrap();
        assert!(dictionary.check("fallbackword") && !dictionary.check("wrongword"));
        assert!(crate::hunspell::source_load_error(slot.name()).is_none());
        let _ = slot.keep(0, Arc::new(dictionary), || {});
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
            let loaded = load_from(&slot, Some((aff.clone(), dic.clone())), &packs);
            assert!(loaded.check("embeddedword") && !loaded.check("nativeword"));
            let (source, reason) = crate::hunspell::load_error(slot.name()).unwrap();
            assert_eq!(source, dic);
            assert!(!reason.is_empty());
            assert_eq!(crate::dictionary_load_error(crate::ProofLang::Pl), Some(reason));
        }
        assert!(load_from(&slot, Some((aff.clone(), dic.clone())), &[]).is_empty());
        let (source, error) = crate::hunspell::load_error(slot.name()).unwrap();
        assert_eq!(source, dic);
        assert!(!error.is_empty());
        std::fs::write(&aff, "SET UTF-8\n").unwrap();
        let loaded = load_from(&slot, Some((aff, dic)), &packs);
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
        let dictionary = load_from(&slot, Some((aff, dic.clone())), &packs);
        assert!(dictionary.check("fallbackword") && !dictionary.check("wrongword"));
        let (file, reason) = crate::hunspell::load_error(slot.name()).unwrap();
        assert_eq!(file, dic);
        assert!(reason.contains("no words"));
        assert_eq!(crate::dictionary_load_error(crate::ProofLang::Pl), Some(reason));
        let _ = slot.keep(0, Arc::new(dictionary), || {});
        assert!(slot.has_dictionary());
        crate::hunspell::clear_load_error(slot.name());
    }

    /// A load that started before a dictionaries-folder change is not kept after its reload.
    #[test]
    fn a_load_from_before_a_reload_is_not_kept() {
        let slot: DictSlot<AffixDict> = DictSlot::new(&["zz_SLOT_RELOAD_TEST"], "Test");
        let before = slot.read().reloads;
        slot.reload();
        let _ = slot.keep(before, Arc::new(AffixDict::default()), || {});
        assert!(slot.read().dict.is_none(), "the next check loads from the new folder");
        let now = slot.read().reloads;
        let _ = slot.keep(now, Arc::new(AffixDict::default()), || {});
        assert!(slot.read().dict.is_some(), "a load since the reload is kept");
    }

    /// Rejected in-flight loads cannot erase an accepted newer load's error.
    #[test]
    fn memory_pack_load_errors_belong_to_the_accepted_slot_result() {
        crate::hold_dictionary_tests();
        let slot: DictSlot<AffixDict> = DictSlot::new(&["zz_MEMORY_PACK_DIAGNOSTIC_TEST"], "Test");
        let source = crate::hunspell::DictionarySource::Files {
            aff: PathBuf::from("memory-pack-diagnostic.aff"),
            dic: PathBuf::from("memory-pack-diagnostic.dic"),
        }; // Diagnostic labels only: this interleaving control performs no file reads.
        let diagnostic = |error: Option<&str>| Some((source.clone(), error.map_or(Ok(()), |e| Err(e.to_string()))));
        let note = |value| crate::hunspell::publish_load_diagnostic(slot.name(), value);
        let empty = || Arc::new(AffixDict::default());
        let before = slot.read().reloads;
        slot.reload();
        let now = slot.read().reloads;
        slot.keep(now, empty(), || note(diagnostic(Some("current load failed"))));
        assert!(crate::hunspell::source_load_error(slot.name()).unwrap().contains("current load failed"));
        slot.keep(before, empty(), || note(None));
        assert!(crate::hunspell::source_load_error(slot.name()).unwrap().contains("current load failed"), "a stale missing load cannot clear it");
        slot.keep(now, empty(), || note(diagnostic(None)));
        assert!(crate::hunspell::source_load_error(slot.name()).unwrap().contains("current load failed"), "a losing success cannot clear it");
        slot.keep(now, empty(), || note(diagnostic(Some("losing load failed"))));
        assert!(crate::hunspell::source_load_error(slot.name()).unwrap().contains("current load failed"), "a losing error cannot replace it");
        slot.reload();
        let now = slot.read().reloads;
        let word = Arc::new(AffixDict::from_hunspell("", "1\nacceptedword\n", &[]).unwrap());
        assert!(slot.keep(now, word, || note(diagnostic(None))).check("acceptedword"));
        assert!(crate::hunspell::source_load_error(slot.name()).is_none(), "an accepted success clears the error");
        slot.reload();
        let now = slot.read().reloads;
        crate::hunspell::publish_load_diagnostic(slot.name(), diagnostic(Some("previous load failed")));
        slot.keep(now, empty(), || note(None));
        assert!(crate::hunspell::source_load_error(slot.name()).is_none(), "an accepted missing result clears the error");
    }
}
