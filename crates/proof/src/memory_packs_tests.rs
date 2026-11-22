//! Original test data: invented words, two elementary affix rules, Liang patterns and licence
//! text authored here. No downloaded dictionary, predecessor fixture or third-party language data.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::packs::{self, PackFiles};
use crate::{EnglishSource, ProofLang, hyphen};

const AFF: &[u8] = b"SET UTF-8\nSFX A Y 1\nSFX A 0 s .\n";
const NOTICE: &[u8] = b"Original memory-pack fixture; no third-party language data.\n";

fn files(locale: &str, word: &str) -> PackFiles {
    PackFiles {
        pack_toml: format!("locale = \"{locale}\"\nname = \"Original {locale}\"\nlicence = \"MIT OR Apache-2.0\"\n").into_bytes(),
        spelling_aff: Some(AFF.to_vec()),
        spelling_dic: Some(format!("1\n{word}/A\n").into_bytes()),
        hyphenation: Some(b".xep1nuq.".to_vec()),
        hyphenation_nonstandard: None,
        licences: vec![("COPYING".into(), NOTICE.to_vec())],
    }
}

/// Own only this test's temporary folder and memory entries; restore the exact prior settings.
struct Fixture {
    root: PathBuf,
    dictionaries: PathBuf,
    disk_packs: PathBuf,
    old_dictionaries: Option<PathBuf>,
    old_packs: Option<PathBuf>,
    old_isolated: bool,
    locales: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        crate::hold_dictionary_tests();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("craft-proof-memory-{}-{unique}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&root).unwrap();
        let dictionaries = root.join("dictionaries");
        let disk_packs = root.join("packs");
        let out = Self {
            root,
            dictionaries,
            disk_packs,
            old_dictionaries: crate::hunspell::dictionary_dir_for_test(),
            old_packs: packs::packs_dir(),
            old_isolated: crate::hunspell::set_isolated_search_for_test(true),
            locales: Vec::new(),
        };
        std::fs::create_dir(&out.dictionaries).unwrap();
        std::fs::create_dir(&out.disk_packs).unwrap();
        crate::set_dictionaries_folder(Some(out.dictionaries.clone()));
        packs::set_packs_dir(Some(out.disk_packs.clone()));
        out
    }

    fn remember(&mut self, locale: &str) {
        if !self.locales.iter().any(|x| x == locale) {
            self.locales.push(locale.to_string());
        }
    }

    fn add(&mut self, input: PackFiles) -> packs::InstalledPack {
        let pack = packs::add_from_memory(input).unwrap();
        self.remember(&pack.locale);
        pack
    }

    fn disk(&self, locale: &str, input: &PackFiles) -> PathBuf {
        let dir = self.disk_packs.join(locale);
        std::fs::create_dir_all(dir.join("LICENCES")).unwrap();
        std::fs::write(dir.join("pack.toml"), &input.pack_toml).unwrap();
        for (name, bytes) in [
            ("spelling.aff", input.spelling_aff.as_ref()),
            ("spelling.dic", input.spelling_dic.as_ref()),
            ("hyphenation.pat", input.hyphenation.as_ref()),
            ("hyphenation.nonstandard", input.hyphenation_nonstandard.as_ref()),
        ] {
            if let Some(bytes) = bytes {
                std::fs::write(dir.join(name), bytes).unwrap();
            }
        }
        for (name, bytes) in &input.licences {
            std::fs::write(dir.join("LICENCES").join(name), bytes).unwrap();
        }
        dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        crate::hunspell::set_isolated_search_for_test(self.old_isolated);
        for locale in &self.locales {
            packs::remove_from_memory(locale);
        }
        packs::set_packs_dir(self.old_packs.take());
        crate::set_dictionaries_folder(self.old_dictionaries.take());
        // root was created exclusively by this Fixture; no borrowed folder is removed.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Debug, PartialEq)]
struct Observed {
    stem: bool,
    suffix: bool,
    suggestions: Vec<String>,
    issues: Vec<crate::Issue>,
    hyphenation: (Vec<usize>, hyphen::Source),
    available: bool,
    error: Option<String>,
}

fn observe(lang: ProofLang, stem: &str, suffix: &str, typo: &str) -> Observed {
    Observed {
        stem: crate::is_correct_in(stem, lang),
        suffix: crate::is_correct_in(suffix, lang),
        suggestions: crate::suggest_in(typo, 8, lang),
        issues: crate::check_spelling_in(&format!("{stem} zzmissing"), lang),
        hyphenation: hyphen::word_breaks_in(stem, lang),
        available: crate::has_dictionary(Some(lang.tag().as_ref())),
        error: crate::dictionary_load_error(lang),
    }
}

fn own_licences() -> Vec<(String, String)> {
    packs::licence_texts().into_iter().filter(|(_, text)| text.as_bytes() == NOTICE).collect()
}

#[test]
fn memory_pack_disk_and_memory_have_identical_public_behavior() {
    let mut f = Fixture::new();
    let lang = ProofLang::from_tag(Some("qaa"));
    let input = files("qaa", "xepnuq");
    let disk = f.disk("qaa", &input);
    packs::reload();
    let on_disk = observe(lang, "xepnuq", "xepnuqs", "xepnqu");
    assert!(on_disk.stem && on_disk.suffix && on_disk.available);
    assert!(on_disk.suggestions.iter().any(|x| x == "xepnuq"));
    assert_eq!(on_disk.issues.len(), 1);
    assert_eq!(on_disk.issues[0].kind, crate::IssueKind::Spelling);
    assert_eq!(on_disk.hyphenation, (vec![3], hyphen::Source::Patterns));
    assert!(on_disk.error.is_none());
    assert_eq!(crate::pack_dictionary_file(lang), Some(disk.join("spelling.dic")));
    let licences = own_licences();
    assert_eq!(licences.len(), 1);
    assert_eq!(packs::installed()[0].path.as_deref(), Some(disk.as_path()));
    let generation = crate::dictionary_generation();
    let memory = f.add(input);
    assert!(crate::dictionary_generation() > generation);
    assert_eq!((memory.locale.as_str(), memory.name.as_str(), memory.licence.as_str()), ("qaa", "Original qaa", "MIT OR Apache-2.0"));
    assert_eq!(memory.path, None);
    assert_eq!(packs::installed(), vec![memory]);
    assert_eq!(observe(lang, "xepnuq", "xepnuqs", "xepnqu"), on_disk);
    assert_eq!(crate::pack_dictionary_file(lang), None, "available memory data has no fabricated file");
    assert_eq!(crate::missing_dictionary(Some("qaa")), None);
    assert_eq!(own_licences(), licences, "same-locale memory does not duplicate disk notices");
    assert!(packs::load_errors().is_empty());
}

#[test]
fn memory_pack_dedicated_and_byte_flag_loaders_match_disk() {
    let mut f = Fixture::new();
    let croatian = files("hr", "xepnuq");
    f.disk("hr", &croatian);
    // The high-byte affix flag is intentionally ISO8859-9, not UTF-8 bytes or a numeric flag.
    let mut turkish = files("tr", "unused");
    turkish.spelling_aff = Some(b"SET ISO8859-9\nSFX \xD0 Y 1\nSFX \xD0 0 lar .\n".to_vec());
    turkish.spelling_dic = Some(b"1\nkutu/\xD0\n".to_vec());
    turkish.hyphenation = Some(b"ku1tu".to_vec());
    f.disk("tr", &turkish);
    packs::reload();
    let hr = observe(ProofLang::Hr, "xepnuq", "xepnuqs", "xepnqu");
    let tr = observe(ProofLang::Tr, "kutu", "kutular", "kutulra");
    assert!(hr.stem && hr.suffix && hr.available && hr.error.is_none());
    assert!(tr.stem && tr.suffix && tr.available && tr.error.is_none());
    f.add(croatian);
    f.add(turkish);
    assert_eq!(observe(ProofLang::Hr, "xepnuq", "xepnuqs", "xepnqu"), hr);
    assert_eq!(observe(ProofLang::Tr, "kutu", "kutular", "kutulra"), tr);
    assert_eq!(packs::installed().len(), 2);
    assert!(packs::installed().iter().all(|x| x.path.is_none()));
}

#[test]
fn memory_pack_changes_invalidate_missing_loaded_caches_and_do_not_mix_origins() {
    let mut f = Fixture::new();
    let lang = ProofLang::from_tag(Some("qaa"));
    assert!(!crate::has_dictionary(Some("qaa")), "cache the missing result first");
    let _ = hyphen::word_breaks_in("xepnuq", lang);
    f.disk("qaa", &files("qaa", "xepnuq"));
    // Do not reload the cached missing result: registration itself must invalidate it.
    let g = crate::dictionary_generation();
    f.add(files("qaa", "vornel"));
    assert!(crate::dictionary_generation() > g);
    assert!(crate::is_correct_in("vornel", lang));
    assert!(!crate::is_correct_in("xepnuq", lang), "the memory dictionary wins as a whole");
    let g = crate::dictionary_generation();
    f.add(files("qaa", "xepnuq"));
    assert!(crate::dictionary_generation() > g);
    assert!(crate::is_correct_in("xepnuq", lang));
    assert!(!crate::is_correct_in("vornel", lang), "replacement invalidates the loaded memory dictionary");
    let mut patterns_only = files("qaa", "unused");
    patterns_only.spelling_aff = None;
    patterns_only.spelling_dic = None;
    patterns_only.hyphenation = Some(b".xepn1uq.".to_vec());
    f.add(patterns_only);
    assert!(!crate::has_dictionary(Some("qaa")), "do not borrow the shadowed disk pack's dictionary");
    assert_eq!(crate::pack_dictionary_file(lang), None);
    assert_eq!(hyphen::word_breaks_in("xepnuq", lang), (vec![4], hyphen::Source::Patterns));
    let g = crate::dictionary_generation();
    assert!(packs::remove_from_memory("qaa"));
    assert!(crate::dictionary_generation() > g);
    assert!(crate::is_correct_in("xepnuq", lang), "removal reveals the disk pack again");
    assert_eq!(hyphen::word_breaks_in("xepnuq", lang), (vec![3], hyphen::Source::Patterns));
}

#[test]
fn memory_pack_reload_and_directory_changes_retain_registered_bytes() {
    let mut f = Fixture::new();
    let lang = ProofLang::from_tag(Some("qaa"));
    f.add(files("qaa", "xepnuq"));
    assert!(crate::is_correct_in("xepnuq", lang));
    let mut generation = crate::dictionary_generation();
    packs::reload();
    assert!(crate::dictionary_generation() > generation);
    generation = crate::dictionary_generation();
    packs::set_packs_dir(None);
    assert!(crate::dictionary_generation() > generation);
    assert!(crate::has_dictionary(Some("qaa")) && crate::is_correct_in("xepnuqs", lang));
    assert_eq!(hyphen::word_breaks_in("xepnuq", lang), (vec![3], hyphen::Source::Patterns));
    let other = f.root.join("other-packs");
    std::fs::create_dir(&other).unwrap();
    generation = crate::dictionary_generation();
    packs::set_packs_dir(Some(other));
    assert!(crate::dictionary_generation() > generation);
    crate::set_dictionaries_folder(Some(f.dictionaries.clone()));
    assert!(crate::is_correct_in("xepnuq", lang));
    assert_eq!(hyphen::word_breaks_in("xepnuq", lang), (vec![3], hyphen::Source::Patterns));
    assert_eq!(packs::installed().len(), 1);
    assert!(packs::installed()[0].path.is_none());
    let g = crate::dictionary_generation();
    assert!(!packs::remove_from_memory("qab"));
    assert_eq!(crate::dictionary_generation(), g, "unknown removal does not invalidate caches");
}

fn loose_dictionary(dir: &Path, name: &str, word: &str) {
    std::fs::write(dir.join(format!("{name}.aff")), AFF).unwrap();
    std::fs::write(dir.join(format!("{name}.dic")), format!("1\n{word}/A\n")).unwrap();
}

#[test]
fn memory_pack_dictionaries_folder_still_has_spelling_priority() {
    let mut f = Fixture::new();
    let lang = ProofLang::from_tag(Some("qaa"));
    f.add(files("qaa", "xepnuq"));
    assert!(crate::is_correct_in("xepnuq", lang));
    loose_dictionary(&f.dictionaries, "qaa", "vornel");
    crate::set_dictionaries_folder(Some(f.dictionaries.clone()));
    assert!(crate::is_correct_in("vornels", lang));
    assert!(!crate::is_correct_in("xepnuq", lang));
    assert_eq!(crate::pack_dictionary_file(lang), Some(f.dictionaries.join("qaa.dic")));
    assert_eq!(hyphen::word_breaks_in("xepnuq", lang), (vec![3], hyphen::Source::Patterns));
    std::fs::remove_file(f.dictionaries.join("qaa.aff")).unwrap();
    std::fs::remove_file(f.dictionaries.join("qaa.dic")).unwrap();
    crate::set_dictionaries_folder(Some(f.dictionaries.clone()));
    assert!(crate::is_correct_in("xepnuq", lang));
    assert_eq!(crate::pack_dictionary_file(lang), None);
}

#[test]
fn memory_pack_english_keeps_the_existing_scowl_and_user_folder_order() {
    let mut f = Fixture::new();
    let baseline = crate::english_source(ProofLang::En);
    let input = files("en-us", "xepnuq");
    f.disk("en-us", &input);
    packs::reload();
    let disk_source = crate::english_source(ProofLang::En);
    let disk_word = crate::is_correct_in("xepnuq", ProofLang::En);
    f.add(input);
    if baseline == Some(EnglishSource::Embedded) {
        assert_eq!(disk_source, baseline, "SCOWL still precedes a disk pack");
        assert_eq!(crate::english_source(ProofLang::En), baseline, "and the same bytes in memory");
        assert_eq!(crate::is_correct_in("xepnuq", ProofLang::En), disk_word);
    } else {
        assert_eq!(crate::english_source(ProofLang::En), Some(EnglishSource::MemoryPack("en-us".into())));
        assert!(disk_word && crate::is_correct_in("xepnuqs", ProofLang::En));
    }
    loose_dictionary(&f.dictionaries, "en_US", "vornel");
    crate::set_dictionaries_folder(Some(f.dictionaries.clone()));
    assert_eq!(crate::english_source(ProofLang::En), Some(EnglishSource::Hunspell(f.dictionaries.join("en_US.dic"))));
    assert!(crate::is_correct_in("vornels", ProofLang::En));
    assert!(!crate::is_correct_in("xepnuq", ProofLang::En));
}

#[test]
fn memory_pack_failed_registration_preserves_the_previous_pack() {
    let mut f = Fixture::new();
    let lang = ProofLang::from_tag(Some("qaa"));
    f.add(files("qaa", "xepnuq"));
    assert!(crate::is_correct_in("xepnuq", lang));
    let before = packs::installed();
    let errors = packs::load_errors();
    let mut invalid = Vec::new();
    let mut input = files("qaa", "vornel");
    input.pack_toml = b"[".to_vec();
    invalid.push(("malformed manifest", input));
    let mut input = files("qaa", "vornel");
    input.pack_toml = b"locale = \"qaa\"\n".to_vec();
    invalid.push(("missing licence", input));
    invalid.push(("noncanonical locale", files("QAA", "vornel")));
    let mut input = files("qaa", "vornel");
    input.pack_toml.extend_from_slice(b"path = \"../escape\"\n");
    invalid.push(("manifest path outside pack", input));
    let mut input = files("qaa", "vornel");
    input.licences[0].0 = "../escape".into();
    invalid.push(("licence path outside pack", input));
    let mut input = files("qaa", "vornel");
    input.spelling_aff = None;
    invalid.push(("missing aff", input));
    let mut input = files("qaa", "vornel");
    input.spelling_dic = None;
    invalid.push(("missing dic", input));
    let mut input = files("qaa", "vornel");
    input.spelling_aff = Some(b"SET UNSUPPORTED-TEST-ENCODING\n".to_vec());
    invalid.push(("unsupported encoding", input));
    let mut input = files("qaa", "vornel");
    input.hyphenation = Some(Vec::new());
    invalid.push(("no patterns", input));
    let mut input = files("qaa", "vornel");
    input.hyphenation = Some(vec![0xFF]);
    invalid.push(("patterns not UTF-8", input));
    let mut input = files("qaa", "vornel");
    input.pack_toml.resize((1 << 20) + 1, b' ');
    invalid.push(("manifest per-file cap", input));
    let mut input = files("qaa", "vornel");
    input.licences[0].1.resize((4 << 20) + 1, b'L');
    invalid.push(("licence per-file cap", input));
    for (label, input) in invalid {
        let generation = crate::dictionary_generation();
        let reason = packs::add_from_memory(input).expect_err(label);
        assert!(!reason.is_empty(), "{label}");
        assert_eq!(packs::installed(), before, "{label}");
        assert_eq!(packs::load_errors(), errors, "failed registration is not a disk load error: {label}");
        assert_eq!(crate::dictionary_generation(), generation, "{label}");
        assert!(crate::is_correct_in("xepnuq", lang), "old loaded dictionary remains: {label}");
        assert!(!crate::is_correct_in("vornel", lang), "failed replacement was not published: {label}");
    }
    // A syntactically valid but wordless English pair must not replace a working pack.
    f.add(files("en-us", "xepnuq"));
    let english_before = observe(ProofLang::En, "xepnuq", "xepnuqs", "xepnqu");
    let source_before = crate::english_source(ProofLang::En);
    let installed_before = packs::installed();
    let licences_before = packs::licence_texts();
    let errors_before = packs::load_errors();
    let generation = crate::dictionary_generation();
    let mut empty = files("en-us", "vornel");
    empty.spelling_dic = Some(b"0\n".to_vec());
    assert!(packs::add_from_memory(empty).is_err(), "an empty English dictionary is not a valid replacement");
    assert_eq!(crate::dictionary_generation(), generation);
    assert_eq!(packs::installed(), installed_before);
    assert_eq!(packs::licence_texts(), licences_before);
    assert_eq!(packs::load_errors(), errors_before);
    assert_eq!(crate::english_source(ProofLang::En), source_before);
    assert_eq!(observe(ProofLang::En, "xepnuq", "xepnuqs", "xepnqu"), english_before, "the loaded English source is unchanged");
}

fn raw_bytes(input: &PackFiles) -> usize {
    input.pack_toml.len()
        + input.spelling_aff.as_ref().map_or(0, Vec::len)
        + input.spelling_dic.as_ref().map_or(0, Vec::len)
        + input.hyphenation.as_ref().map_or(0, Vec::len)
        + input.hyphenation_nonstandard.as_ref().map_or(0, Vec::len)
        + input.licences.iter().map(|(name, text)| name.len() + text.len()).sum::<usize>()
}

#[test]
fn memory_pack_raw_budget_counts_all_inputs_and_is_released() {
    let mut f = Fixture::new();
    let mut a = files("qaa", "xepnuq");
    a.licences[0].1 = vec![b'A'; 700];
    let size = raw_bytes(&a);
    assert!(size < 1024 && size * 2 > 1024);
    f.remember("qaa");
    f.remember("qab");
    let generation = crate::dictionary_generation();
    assert!(packs::add_from_memory_with_limit_for_test(a, size - 1).is_err());
    assert!(packs::installed().is_empty());
    assert_eq!(crate::dictionary_generation(), generation);
    let mut a = files("qaa", "xepnuq");
    a.licences[0].1 = vec![b'A'; 700];
    packs::add_from_memory_with_limit_for_test(a, size).unwrap();
    let mut b = files("qab", "vornel");
    b.licences[0].1 = vec![b'B'; 700];
    let generation = crate::dictionary_generation();
    assert!(packs::add_from_memory_with_limit_for_test(b, 1024).is_err(), "licence bytes count in the shared retained budget");
    assert_eq!(crate::dictionary_generation(), generation);
    assert_eq!(packs::installed().iter().map(|x| x.locale.as_str()).collect::<Vec<_>>(), ["qaa"]);
    assert!(packs::remove_from_memory("qaa"));
    let mut b = files("qab", "vornel");
    b.licences[0].1 = vec![b'B'; 700];
    packs::add_from_memory_with_limit_for_test(b, 1024).unwrap();
    assert_eq!(packs::installed().iter().map(|x| x.locale.as_str()).collect::<Vec<_>>(), ["qab"]);
    assert!(crate::has_dictionary(Some("qab")));
}

/// Original whole-word records: the ordinary point is after "as"; the replacement adds only
/// the display letter "z" before the hyphen. No language dictionary or pattern corpus is used.
#[test]
fn memory_pack_hungarian_replacements_survive_removal() {
    let mut fixture = Fixture::new();
    let lang = ProofLang::from_tag(Some("hu"));
    let limits = hyphen::Limits::default();
    let input = || {
        let mut input = files("hu", "asszony");
        input.spelling_aff = None;
        input.spelling_dic = None;
        input.hyphenation = Some(b".as1szony.\n".to_vec());
        input
    };
    // A standard-only memory pack must not borrow embedded replacement rules.
    fixture.add(input());
    assert_eq!(hyphen::hyphenate_word_in("asszony", &limits, lang), "as-szony");
    let mut auxiliary_only = input();
    auxiliary_only.hyphenation = None;
    auxiliary_only.hyphenation_nonstandard = Some(b"UTF-8\n.as1szony./sz=,2,1\n".to_vec());
    fixture.add(auxiliary_only);
    assert_eq!(hyphen::hyphenate_word_in("asszony", &limits, lang), "asz-szony");
    let mut replacement = input();
    replacement.hyphenation_nonstandard = Some(b"UTF-8\n.as1szony./sz=,2,1\n".to_vec());
    fixture.add(replacement);
    assert_eq!(hyphen::hyphenate_word_in("asszony", &limits, lang), "asz-szony");
    eprintln!("memory_pack_hu: memory_verified");
    assert!(packs::remove_from_memory("hu"));
    if packs::embedded_locales().contains(&"hu") {
        assert_eq!(hyphen::hyphenate_word_in("asszony", &limits, lang), "asz-szony", "removal reveals the embedded Hungarian replacement rules");
        eprintln!("memory_pack_hu: embedded_fallback_verified");
    } else {
        eprintln!("memory_pack_hu: embedded_fallback_absent; build with CRAFT_PROOF_EMBED_PACKS=hu and the original tiny pack fixture");
    }
}
