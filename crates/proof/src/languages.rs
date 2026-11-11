//! The table of languages proofed against Hunspell dictionaries, and the dictionary-folder and load-error API.

use super::*;

/// A language proofed against an installed Hunspell dictionary: all the functions below need to
/// know about it, so a language joins with one row in [`LANGUAGES`].
pub(super) struct Language {
    pub(super) lang: ProofLang,
    pub(super) slot: &'static dyn slot::Slot,
    /// The spelling mark's message, in the language.
    pub(super) message: &'static str,
    pub(super) is_correct: fn(&str, ProofLang, &Options) -> bool,
    pub(super) suggest: fn(&str, usize, ProofLang) -> Vec<String>,
    /// Grammar issues; the flag says whether the first word must have a capital (not in a list
    /// item).
    pub(super) grammar: fn(&str, ProofLang, bool) -> Vec<Issue>,
}

/// Every language proofed against a Hunspell dictionary, in the order File › Options › Proofing
/// lists them. English has a built-in word list and is handled apart.
pub(super) static LANGUAGES: [Language; 12] = [
    Language {
        lang: ProofLang::Pl,
        slot: &pl::DICT,
        message: "Możliwy błąd pisowni",
        is_correct: |w, _, o| pl::is_correct_with_options(w, o),
        suggest: |w, n, _| pl::suggest(w, n),
        grammar: |t, _, c| pl::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::Pt,
        slot: &pt::DICT,
        message: "Possível erro de ortografia",
        is_correct: |w, _, o| pt::is_correct_with_options(w, o),
        suggest: |w, n, _| pt::suggest(w, n),
        grammar: |t, _, c| pt::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::It,
        slot: &it::DICT,
        message: "Possibile errore di ortografia",
        is_correct: |w, _, o| it::is_correct_with_options(w, o),
        suggest: |w, n, _| it::suggest(w, n),
        grammar: |t, _, c| it::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::Hr,
        slot: &hr::DICT,
        message: "Moguća pravopisna pogreška",
        is_correct: |w, _, o| hr::is_correct_with_options(w, o),
        suggest: |w, n, _| hr::suggest(w, n),
        grammar: |t, _, c| hr::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::Tr,
        slot: &tr::DICT,
        message: "Olası yazım hatası",
        is_correct: |w, _, o| tr::is_correct_with_options(w, o),
        suggest: |w, n, _| tr::suggest(w, n),
        grammar: |t, _, c| tr::grammar::check_with(t, c),
    },
    Language {
        lang: ProofLang::Ru,
        slot: &ru::DICT,
        message: "Возможна орфографическая ошибка",
        is_correct: |w, _, o| ru::is_correct_with_options(w, o),
        suggest: |w, n, _| ru::suggest(w, n),
        grammar: |t, _, c| ru::grammar::check_with(t, c),
    },
    // Czech, Indonesian and Spanish have no sentence-capital rule, so a list item needs no
    // exception.
    Language {
        lang: ProofLang::Cs,
        slot: &cs::DICT,
        message: "Možná pravopisná chyba",
        is_correct: |w, _, o| cs::is_correct_with_options(w, o),
        suggest: |w, n, _| cs::suggest(w, n),
        grammar: |t, _, _| cs::grammar::check(t),
    },
    Language {
        lang: ProofLang::Id,
        slot: &id::DICT,
        message: "Kemungkinan salah eja",
        is_correct: |w, _, o| id::is_correct_with_options(w, o),
        suggest: |w, n, _| id::suggest(w, n),
        grammar: |t, _, _| id::grammar::check(t),
    },
    Language {
        lang: ProofLang::Es,
        slot: &es::SPAIN_DICT,
        message: "Posible error ortográfico",
        is_correct: es::is_correct_with_options,
        suggest: es::suggest,
        grammar: |t, l, _| es::grammar::check(t, l),
    },
    Language {
        lang: ProofLang::EsMx,
        slot: &es::MEXICO_DICT,
        message: "Posible error ortográfico",
        is_correct: es::is_correct_with_options,
        suggest: es::suggest,
        grammar: |t, l, _| es::grammar::check(t, l),
    },
    Language {
        lang: ProofLang::Es419,
        slot: &es::LATIN_AMERICA_DICT,
        message: "Posible error ortográfico",
        is_correct: es::is_correct_with_options,
        suggest: es::suggest,
        grammar: |t, l, _| es::grammar::check(t, l),
    },
    Language {
        lang: ProofLang::EsUy,
        slot: &es::URUGUAY_DICT,
        message: "Posible error ortográfico",
        is_correct: es::is_correct_with_options,
        suggest: es::suggest,
        grammar: |t, l, _| es::grammar::check(t, l),
    },
];

/// `lang`'s row in [`LANGUAGES`]; `None` for English and the languages WordCraft doesn't proof.
pub(super) fn language(lang: ProofLang) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.lang == lang)
}

/// Search `dir` for Hunspell dictionaries before the system locations (File › Options ›
/// Proofing › Dictionaries folder, #1063); `None` clears it. The dictionaries load again from
/// the new search path, and text is checked again.
pub fn set_dictionaries_folder(dir: Option<std::path::PathBuf>) {
    hunspell::set_dictionary_dir(dir);
    reload_dictionaries();
}

/// Forget every loaded dictionary, so the next check looks for them again (the dictionaries
/// folder or the installed Language packs changed), and check text again.
pub(crate) fn reload_dictionaries() {
    for l in &LANGUAGES {
        l.slot.reload();
    }
    english::reload();
    pack_lang::reload();
    DICTIONARY_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The `.dic` file a [`ProofLang::Pack`] language's (or German's or Bulgarian's) spelling is read from now (the dictionaries
/// folder, an installed Language pack or the system's); `None` when it has none (unchecked) or
/// `lang` isn't a pack language. Reads the dictionary once (it is cached afterwards).
pub fn pack_dictionary_file(lang: ProofLang) -> Option<std::path::PathBuf> {
    pack_lang::dictionary_file(lang.spelling_locale()?)
}

/// Each language that reads an installed Hunspell dictionary, with the `.dic` file found for it
/// now (`None`: not installed, so the language has no spelling marks).
pub fn installed_dictionaries() -> Vec<(ProofLang, Option<std::path::PathBuf>)> {
    LANGUAGES.iter().map(|l| (l.lang, l.slot.installed_path())).collect()
}

/// Why the dictionary found for `lang` failed to load (unsupported encoding, too large, a parse
/// error), if it was read and failed. Doesn't read anything itself: a dictionary nobody has
/// checked yet reports `None` (#2830). For an English, its embedded SCOWL dictionary or the
/// English Hunspell dictionary that failed; English then uses the next source ([`english_source`]).
pub fn dictionary_load_error(lang: ProofLang) -> Option<String> {
    if lang.is_english() {
        return english::load_error(lang);
    }
    if let Some(l) = lang.spelling_locale() {
        return pack_lang::load_error(l);
    }
    hunspell::load_error(language(lang)?.slot.name()).map(|(_, reason)| reason)
}

/// Where the spelling of an English `lang` comes from now (`None` for other languages), in this
/// order: a Hunspell dictionary of its own (`en_US`, `en_GB`, `en_CA`) in the dictionaries
/// folder, the SCOWL dictionary embedded at build time, one of its own on the system, else the
/// Moby word list built into the crate. Another English's dictionary is never used. Reads the
/// Hunspell dictionary once (it is cached afterwards).
pub fn english_source(lang: ProofLang) -> Option<EnglishSource> {
    lang.is_english().then(|| english::source(lang))
}
