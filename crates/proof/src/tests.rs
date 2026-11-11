use super::*;

/// The non-English test dictionaries every dictionaries-folder test puts in its folder, so
/// tests running meanwhile see the same words whichever of those folders is set.
fn write_test_dictionaries(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, aff, dic) in [
        ("pl_PL", pl::test_dict::AFF, pl::test_dict::DIC),
        ("pt_BR", pt::test_dict::AFF, pt::test_dict::DIC),
        ("it_IT", it::test_dict::AFF, it::test_dict::DIC),
        ("hr_HR", hr::test_dict::AFF, hr::test_dict::DIC),
        ("tr_TR", tr::test_dict::AFF, tr::test_dict::DIC),
        ("ru_RU", ru::test_dict::AFF, ru::test_dict::DIC),
        ("cs_CZ", cs::test_dict::AFF, cs::test_dict::DIC),
        ("id_ID", id::test_dict::AFF, id::test_dict::DIC),
        ("es_ES", es::test_dict::AFF, es::test_dict::DIC),
        ("es_MX", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
        ("es_419", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
        ("es_UY", es::test_dict::AFF, es::test_dict::DIC_AMERICAS),
    ] {
        std::fs::write(dir.join(format!("{name}.aff")), aff).unwrap();
        std::fs::write(dir.join(format!("{name}.dic")), dic).unwrap();
    }
}

/// Set `dir` (with [`write_test_dictionaries`] and `english`, `(name, .aff, .dic)` files) as
/// the dictionaries folder; the English files are removed again by [`drop_english_files`].
fn folder_with_english(dir: &std::path::Path, english: &[(&str, &[u8], &[u8])]) {
    write_test_dictionaries(dir);
    for (name, aff, dic) in english {
        std::fs::write(dir.join(format!("{name}.aff")), aff).unwrap();
        std::fs::write(dir.join(format!("{name}.dic")), dic).unwrap();
    }
    set_dictionaries_folder(Some(dir.to_path_buf()));
}

/// Remove the English files of `dir` and set it again: English goes back to what it was
/// (the built-in list, or a dictionary installed on the system), for the tests that follow.
fn drop_english_files(dir: &std::path::Path, english: &[(&str, &[u8], &[u8])]) {
    for (name, _, _) in english {
        let _ = std::fs::remove_file(dir.join(format!("{name}.aff")));
        let _ = std::fs::remove_file(dir.join(format!("{name}.dic")));
    }
    set_dictionaries_folder(Some(dir.to_path_buf()));
}

/// Language-data builds embed SCOWL: an English dictionary in the dictionaries folder would
/// then come before it for every English test running meanwhile, so the folder tests below
/// run without the data only; `english::tests` checks the order with the data.
fn scowl_embedded() -> bool {
    scowl::dict(ProofLang::En).is_some()
}

/// R6.4: an English Hunspell dictionary that is there but can't be read (an encoding we don't
/// read, bytes that aren't UTF-8, no words) is an error reported for its English, never a
/// panic, and that English falls back to the next dictionary or the built-in word list, which
/// still marks `becuase`.
#[test]
fn unreadable_english_dictionary_is_reported_and_skipped() {
    if scowl_embedded() {
        return;
    }
    crate::hold_dictionary_tests();
    let dir = std::env::temp_dir().join(format!("wc-proof-en-unreadable-{}", std::process::id()));
    let english: &[(&str, &[u8], &[u8])] =
        &[("en_US", b"SET UTF-8\n", b"0\n"), ("en_GB", b"SET SHIFT_JIS\n", b"1\ncolour\n"), ("en_CA", b"SET UTF-8\n", b"1\n\xff\xfe\n")];
    folder_with_english(&dir, english);
    for (lang, reason) in [(ProofLang::En, "no words"), (ProofLang::EnGb, "unsupported encoding"), (ProofLang::EnCa, "not UTF-8")] {
        {
            // The built-in list, or a dictionary installed on the system (tried next).
            let source = english_source(lang);
            assert!(!matches!(&source, Some(EnglishSource::Hunspell(p)) if p.starts_with(&dir)), "{lang:?}: {source:?}");
            let error = dictionary_load_error(lang).unwrap_or_default();
            assert!(error.contains(reason), "{lang:?}: {error}");
        }
        assert!(!is_correct_in("becuase", lang) && is_correct_in("because", lang) && is_correct_in("houses", lang), "{lang:?}");
        assert!(suggest_in("becuase", 3, lang).contains(&"because".to_string()), "{lang:?}");
    }
    drop_english_files(&dir, english);
    assert_eq!(dictionary_load_error(ProofLang::EnGb), None, "a new folder forgets the old errors");
}

/// The test dictionaries' folder the dictionaries-folder tests leave set for the rest of the run.
fn shared_test_folder() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wc-proof-dictionaries-{}", std::process::id()));
    write_test_dictionaries(&dir);
    dir
}

/// Write a Language pack folder `<packs>/<locale>/` as the font store unpacks one.
fn write_pack(packs: &std::path::Path, locale: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
    let dir = packs.join(locale);
    std::fs::create_dir_all(dir.join("LICENCES")).unwrap();
    std::fs::write(dir.join("pack.toml"), format!("locale = \"{locale}\"\nname = \"Hrvatski (test)\"\nlicence = \"MIT\"\nclass = \"Embeddable\"\nsources = [\"https://example.org/{locale}\"]\n")).unwrap();
    std::fs::write(dir.join("LICENCES").join("COPYING"), "Test licence text.").unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    dir
}

/// CP4.2: a language with no tools of its own (Dutch) is spell-checked, given suggestions and
/// hyphenated from its installed Language pack; removed, it is unchecked again; English is
/// unaffected. No grammar rules for it.
#[test]
fn a_pack_proofs_a_language_without_its_own_tools() {
    crate::hold_dictionary_tests();
    // The test dictionaries on disk first, so a reload never leaves other tests without them.
    set_dictionaries_folder(Some(shared_test_folder()));
    let root = std::env::temp_dir().join(format!("wc-proof-nl-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let nl = ProofLang::from_tag(Some("nl-NL"));
    assert_eq!(nl.pack_locale().map(|l| l.to_string()).as_deref(), Some("nl-nl"));
    let text = "Het huis en de fietz.";
    let english_before = check_text(text, &[]);
    let unchecked = |lang: ProofLang| check_text(text, &[(0..text.len(), lang)]).iter().all(|i| i.kind == IssueKind::Grammar);
    let before = (is_correct_in("fietz", nl), hyphen::hyphenate_word_in("xxnaxx", &hyphen::Limits::default(), nl), pack_dictionary_file(nl));
    let pack = write_pack(
        &root,
        "nl",
        &[
            ("spelling.aff", b"SET UTF-8\nTRY esianrtolducgmphbykfvwjzxq\nSFX E Y 1\nSFX E 0 en .\n"),
            ("spelling.dic", b"5\nhet\nhuis/E\nen\nde\nfiets/E\n"),
            ("hyphenation.pat", b"1na xx\n"),
        ],
    );
    packs::set_packs_dir(Some(root.clone()));
    assert!(nl.is_proofed() && has_dictionary(Some("nl-BE")) && !has_dictionary(Some("fr-FR")));
    assert_eq!(pack_dictionary_file(nl), Some(pack.join("spelling.dic")));
    assert_eq!(pack_dictionary_file(ProofLang::from_tag(Some("nl"))), Some(pack.join("spelling.dic")), "a region falls back to the pack");
    for w in ["huis", "Huis", "huizen", "huisen", "fietsen", "HUIS"] {
        assert_eq!(is_correct_in(w, nl), w != "huizen", "{w}");
    }
    let issues = check_text(text, &[(0..text.len(), nl)]);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!((issues[0].kind, &text[issues[0].start..issues[0].end], issues[0].lang), (IssueKind::Spelling, "fietz", nl));
    assert_eq!(issues[0].message, "Mogelijke spelfout", "the language's own message");
    assert_eq!((missing_dictionary(Some("nl-NL")), unreadable_dictionary(Some("nl-NL"))), (None, None));
    let missing_fr = missing_dictionary(Some("fr-FR"));
    assert!(missing_fr.is_none_or(|n| n == "fr_FR"), "{missing_fr:?}");
    assert!(suggest_in("fietz", 5, nl).contains(&"fiets".to_string()), "{:?}", suggest_in("fietz", 5, nl));
    assert!(check_grammar_in("de de huis", nl).is_empty(), "no grammar rules");
    assert_eq!(hyphen::hyphenate_word_in("xxnaxx", &hyphen::Limits::default(), nl), "xx-naxx");
    assert!(unchecked(ProofLang::from_tag(Some("fr-FR"))), "a language with no pack stays unchecked");
    assert_eq!(check_text(text, &[]), english_before, "English is unaffected");
    // A broken dictionary is reported, never a panic.
    std::fs::write(pack.join("spelling.aff"), "SET SHIFT_JIS\n").unwrap();
    packs::reload();
    assert!(is_correct_in("fietz", nl) && !nl.is_proofed());
    assert!(dictionary_load_error(nl).unwrap_or_default().contains("unsupported encoding"));
    let (name, file, reason) = unreadable_dictionary(Some("nl-NL")).unwrap();
    assert_eq!((name, file), ("nl_NL", pack.join("spelling.dic")));
    assert!(reason.contains("unsupported encoding"), "{reason}");
    assert_eq!(missing_dictionary(Some("nl-NL")), Some("nl_NL"), "as for the other languages");
    // Removed: unchecked again.
    std::fs::remove_dir_all(&pack).unwrap();
    packs::reload();
    assert_eq!((is_correct_in("fietz", nl), hyphen::hyphenate_word_in("xxnaxx", &hyphen::Limits::default(), nl), pack_dictionary_file(nl)), before);
    assert!(unchecked(nl) || before.2.is_some());
    assert_eq!(check_text(text, &[]), english_before);
    packs::set_packs_dir(None);
    set_dictionaries_folder(Some(shared_test_folder()));
    let _ = std::fs::remove_dir_all(&root);
}

/// The `.dic` file `lang`'s spelling is read from with the installed packs and `system` as the
/// system's only dictionary folder.
fn chosen_dictionary(lang: ProofLang, system: &std::path::Path) -> Option<std::path::PathBuf> {
    let names: Vec<String> = match lang.spelling_locale() {
        Some(l) => pack_lang::names(l),
        None => language(lang).map(|l| l.slot.names().iter().map(|n| (*n).to_string()).collect()).unwrap_or_default(),
    };
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (region, other) = packs::spelling_dirs(&refs);
    hunspell::ordered(&refs, &[], &region, &[system.to_path_buf()], &other).into_iter().next().map(|(_, dic)| dic)
}

/// Second review of #485: the bare `es` and `de` packs are Spain's and Germany's, a region's
/// text reads its own country's pack, and the system's dictionary of the right region beats
/// another region's pack.
#[test]
fn regional_text_reads_its_own_regions_pack() {
    crate::hold_dictionary_tests();
    set_dictionaries_folder(Some(shared_test_folder()));
    let root = std::env::temp_dir().join(format!("wc-proof-region-packs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (packs_dir, system) = (root.join("packs"), root.join("system"));
    std::fs::create_dir_all(&system).unwrap();
    let sys = |name: &str| {
        std::fs::write(system.join(format!("{name}.aff")), "SET UTF-8\n").unwrap();
        std::fs::write(system.join(format!("{name}.dic")), "1\nx\n").unwrap();
        system.join(format!("{name}.dic"))
    };
    let pack = |locale: &str| write_pack(&packs_dir, locale, &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", b"1\nx\n")]).join("spelling.dic");
    let tag = |t: &str| ProofLang::from_tag(Some(t));
    packs::set_packs_dir(Some(packs_dir.clone()));

    // es + es-ar + es-mx: each country follows its own list; the bare pack is Spain's.
    let (es, es_ar, es_mx) = (pack("es"), pack("es-ar"), pack("es-mx"));
    packs::reload();
    assert_eq!(chosen_dictionary(tag("es-UY"), &system), Some(es_ar.clone()), "Uruguay: its own, then Argentina's");
    assert_eq!(chosen_dictionary(tag("es-CO"), &system), Some(es_mx.clone()), "Latin America: Mexico's before Argentina's");
    assert_eq!(chosen_dictionary(tag("es-AR"), &system), Some(es_mx.clone()), "es-AR is Latin American Spanish");
    assert_eq!(chosen_dictionary(tag("es-MX"), &system), Some(es_mx.clone()));
    assert_eq!(chosen_dictionary(tag("es-ES"), &system), Some(es.clone()));
    std::fs::remove_dir_all(es_mx.parent().unwrap()).unwrap();
    packs::reload();
    for t in ["es-AR", "es-CO", "es-UY"] {
        assert_eq!(chosen_dictionary(tag(t), &system), Some(es_ar.clone()), "{t}: never Spain's bare pack first");
    }
    std::fs::remove_dir_all(&packs_dir).unwrap();

    // de + de-ch, system de_DE: Swiss text reads the de-ch pack; German text the bare one.
    let sys_de = sys("de_DE");
    let (de, de_ch) = (pack("de"), pack("de-ch"));
    packs::reload();
    assert_eq!(chosen_dictionary(tag("de-CH"), &system), Some(de_ch.clone()));
    assert_eq!(chosen_dictionary(tag("de-LI"), &system), Some(de_ch.clone()));
    assert_eq!(chosen_dictionary(tag("de-DE"), &system), Some(de.clone()), "the bare pack is Germany's: before the system's de_DE");
    std::fs::remove_dir_all(de.parent().unwrap()).unwrap();
    packs::reload();
    assert_eq!(chosen_dictionary(tag("de-AT"), &system), Some(sys_de), "never the Swiss pack");
    std::fs::remove_dir_all(&packs_dir).unwrap();

    // pt-br + pt-pt, or system pt_BR: European text reads the pt-pt pack.
    let (pt_br, pt_pt) = (pack("pt-br"), pack("pt-pt"));
    packs::reload();
    assert_eq!(chosen_dictionary(tag("pt-PT"), &system), Some(pt_pt.clone()));
    assert_eq!(chosen_dictionary(tag("pt-BR"), &system), Some(pt_br.clone()));
    std::fs::remove_dir_all(pt_br.parent().unwrap()).unwrap();
    let sys_pt_br = sys("pt_BR");
    packs::reload();
    assert_eq!(chosen_dictionary(tag("pt-PT"), &system), Some(pt_pt.clone()));
    assert_eq!(chosen_dictionary(tag("pt-BR"), &system), Some(sys_pt_br), "another region's pack after the system's right region");
    std::fs::remove_dir_all(&packs_dir).unwrap();

    // fr pack + system fr_CA: the system's exact region beats another region's pack.
    let sys_fr_ca = sys("fr_CA");
    let fr = pack("fr");
    packs::reload();
    assert_eq!(chosen_dictionary(tag("fr-CA"), &system), Some(sys_fr_ca));
    assert_eq!(chosen_dictionary(tag("fr-FR"), &system), Some(fr.clone()));
    assert_eq!(chosen_dictionary(tag("fr"), &system), Some(fr));

    // Swiss text spells from its pack, hyphenates as German and reads German messages.
    std::fs::remove_dir_all(&packs_dir).unwrap();
    write_pack(&packs_dir, "de", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", "1\nStraße\n".as_bytes())]);
    write_pack(&packs_dir, "de-ch", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", "1\nStrasse\n".as_bytes())]);
    packs::reload();
    let (ch, de) = (tag("de-CH"), tag("de-DE"));
    assert!(is_correct_in("Strasse", ch) && !is_correct_in("Straße", ch));
    assert!(is_correct_in("Straße", de) && !is_correct_in("Strasse", de));
    assert_eq!(
        hyphen::hyphenate_word_in("Silbentrennung", &hyphen::Limits::default(), ch),
        hyphen::hyphenate_word_in("Silbentrennung", &hyphen::Limits::default(), de)
    );
    assert_eq!(spelling_message(ch), "Möglicher Rechtschreibfehler");
    packs::set_packs_dir(None);
    set_dictionaries_folder(Some(shared_test_folder()));
    let _ = std::fs::remove_dir_all(&root);
}

/// Pack languages for the cache tests: `qa`, `qb`… each knowing `word<locale>`.
fn many_packs(root: &std::path::Path, n: usize) -> Vec<ProofLang> {
    (0..n)
        .map(|i| {
            let l = format!("q{}", (b'a' + i as u8) as char);
            write_pack(root, &l, &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", format!("1\nword{l}\n").as_bytes())]);
            ProofLang::from_tag(Some(&l))
        })
        .collect()
}

/// Review of #485: ten pack languages used in turn stay loaded (a memory budget, not eight
/// entries), so switching between them never reads a dictionary again.
#[test]
fn many_pack_languages_load_once() {
    crate::hold_dictionary_tests();
    set_dictionaries_folder(Some(shared_test_folder()));
    let root = std::env::temp_dir().join(format!("wc-proof-many-packs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let langs = many_packs(&root, 10);
    packs::set_packs_dir(Some(root.clone()));
    let loads = pack_lang::loads();
    // A paragraph of 20 words over the ten languages, checked 50 times.
    let mut text = String::new();
    let mut spans = Vec::new();
    for k in 0..20 {
        let lang = langs[k % 10];
        let w = format!("word{} ", lang.pack_locale().unwrap());
        spans.push((text.len()..text.len() + w.len(), lang));
        text.push_str(&w);
    }
    for _ in 0..50 {
        assert!(check_text(&text, &spans).is_empty());
        for lang in &langs {
            assert!(!is_correct_in("wordzz", *lang));
        }
    }
    assert_eq!(pack_lang::loads() - loads, 10, "each dictionary is read once");
    // A language with no dictionary takes no room and is not looked up again.
    assert!(is_correct_in("anything", ProofLang::from_tag(Some("qz"))));
    packs::set_packs_dir(None);
    set_dictionaries_folder(Some(shared_test_folder()));
    let _ = std::fs::remove_dir_all(&root);
}

/// Review of #485: over the memory budget, a dictionary used during the current paragraph is
/// never evicted, so the paragraph loads each once.
#[test]
fn a_paragraph_never_evicts_its_own_dictionaries() {
    crate::hold_dictionary_tests();
    set_dictionaries_folder(Some(shared_test_folder()));
    let root = std::env::temp_dir().join(format!("wc-proof-budget-packs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let langs = many_packs(&root, 2);
    packs::set_packs_dir(Some(root.clone()));
    pack_lang::set_budget(1);
    let (a, b) = (langs[0], langs[1]);
    let text = "wordqa wordqb wordqa wordqb wordqa wordqb";
    let spans: Vec<(Range<usize>, ProofLang)> = (0..6).map(|k| (k * 7..k * 7 + 6, if k % 2 == 0 { a } else { b })).collect();
    let loads = pack_lang::loads();
    assert!(check_text(text, &spans).is_empty());
    assert_eq!(pack_lang::loads() - loads, 2);
    pack_lang::set_budget(pack_lang::MAX_LOADED_BYTES);
    packs::set_packs_dir(None);
    set_dictionaries_folder(Some(shared_test_folder()));
    let _ = std::fs::remove_dir_all(&root);
}

/// CP4.2, hostile packs: a missing or broken `pack.toml`, a path outside the pack, a folder
/// that isn't a language tag, a symbolic link, patterns that aren't UTF-8 or a dictionary that
/// can't be read are reported, never a panic, and the pack (or that file) is skipped.
#[test]
fn corrupt_packs_are_reported_and_skipped() {
    crate::hold_dictionary_tests();
    // The test dictionaries on disk first, so a reload never leaves other tests without them.
    set_dictionaries_folder(Some(shared_test_folder()));
    let root = std::env::temp_dir().join(format!("wc-proof-bad-packs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // Good spelling (the test words, so Croatian tests running meanwhile are unaffected), bad
    // patterns.
    write_pack(
        &root,
        "hr",
        &[("spelling.aff", hr::test_dict::AFF.as_bytes()), ("spelling.dic", hr::test_dict::DIC.as_bytes()), ("hyphenation.pat", b"\xff\xfe a1b")],
    );
    let traversal = write_pack(&root, "nl", &[("spelling.aff", b"SET UTF-8\n"), ("spelling.dic", b"1\nhuis\n")]);
    std::fs::write(traversal.join("pack.toml"), "locale = \"nl\"\nlicence = \"MIT\"\n[[file]]\npath = \"../../../etc/passwd\"\n").unwrap();
    let wrong = write_pack(&root, "fr", &[]);
    std::fs::write(wrong.join("pack.toml"), "locale = \"de\"\nlicence = \"MIT\"\n").unwrap();
    let broken = write_pack(&root, "sk", &[]);
    std::fs::write(broken.join("pack.toml"), "locale = \"sk\n").unwrap();
    std::fs::create_dir_all(root.join("ca")).unwrap();
    std::fs::create_dir_all(root.join("Bad_Name")).unwrap();
    std::fs::create_dir_all(root.join(".tmp-el-1")).unwrap();
    let huge = write_pack(&root, "lt", &[]);
    std::fs::write(huge.join("pack.toml"), vec![b'#'; 2 << 20]).unwrap();
    #[cfg(unix)]
    {
        let linked = write_pack(&root, "sl", &[("hyphenation.pat", b"a1b")]);
        std::fs::remove_file(linked.join("hyphenation.pat")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", linked.join("hyphenation.pat")).unwrap();
        std::os::unix::fs::symlink(&linked, root.join("sv")).unwrap();
    }
    packs::set_packs_dir(Some(root.clone()));
    let locales: Vec<String> = packs::installed().into_iter().map(|p| p.locale).collect();
    assert_eq!(locales, if cfg!(unix) { vec!["hr", "sl"] } else { vec!["hr"] });
    let _ = hyphen::hyphenate_word_in("hyphenation", &hyphen::Limits::default(), ProofLang::Hr);
    let errors = packs::load_errors();
    let reason = |l: &str| errors.iter().find(|(k, _)| k == l).map(|(_, e)| e.clone()).unwrap_or_default();
    assert!(reason("nl").contains("outside the pack"), "{errors:?}");
    assert!(reason("fr").contains("names locale"), "{errors:?}");
    assert!(reason("sk").starts_with("pack.toml"), "{errors:?}");
    assert!(reason("ca").contains("no pack.toml"), "{errors:?}");
    assert!(reason("Bad_Name").contains("language tag"), "{errors:?}");
    assert!(reason("lt").contains("too large"), "{errors:?}");
    assert!(reason("hr").contains("not UTF-8"), "{errors:?}");
    assert!(!errors.iter().any(|(k, _)| k.starts_with('.')), "temporary folders are not packs");
    #[cfg(unix)]
    {
        assert!(reason("sv").contains("not a folder"), "{errors:?}");
        assert!(hunspell::find(&["sl"]).is_none_or(|(_, d)| !d.starts_with(&root)), "a link is never read");
    }
    assert!(is_correct_in("kuća", ProofLang::Hr), "the pack's spelling still works");
    // A dictionary in a pack that can't be read is reported like any other.
    let bad = write_pack(&root, "hr", &[("spelling.aff", b"SET SHIFT_JIS\n"), ("spelling.dic", b"1\nkuca\n")]);
    let (aff, dic) = hunspell::find_in(&bad, &["spelling"]).unwrap();
    assert!(hunspell::load(&aff, &dic).unwrap_err().contains("unsupported encoding"));
    packs::set_packs_dir(None);
    set_dictionaries_folder(Some(shared_test_folder()));
    let _ = std::fs::remove_dir_all(&root);
}

/// Each English accepts its own spellings and flags the other dialect's, suggesting its own
/// first, as Word does (#1281, #1045).
#[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
#[test]
fn english_dialects_flag_each_others_spellings() {
    // English results depend on the dictionaries other tests install.
    crate::hold_dictionary_tests();
    let (us, gb, ca) = (ProofLang::En, ProofLang::EnGb, ProofLang::EnCa);
    // (word, right in en-US, right in en-CA, right in en-GB)
    let cases = [
        ("color", true, false, false),
        ("colour", false, true, true),
        ("colours", false, true, true),
        ("coloured", false, true, true),
        ("Colourful", false, true, true),
        ("colour's", false, true, true),
        ("honorable", true, false, false),
        ("neighbourhood", false, true, true),
        ("center", true, false, false),
        ("centre", false, true, true),
        ("centred", false, true, true),
        ("centring", false, true, true),
        ("centering", true, false, false),
        ("theater", true, false, false),
        ("theatre", true, true, true),
        ("organize", true, true, true),
        ("organise", false, false, true),
        ("organisation", false, false, true),
        ("organisations", false, false, true),
        ("realised", false, false, true),
        ("recognising", false, false, true),
        ("analyze", true, true, false),
        ("analyse", false, true, true),
        ("paralysed", false, true, true),
        ("defense", true, false, false),
        ("defence", false, true, true),
        ("licence", false, true, true),
        ("license", true, true, true),
        ("practise", false, true, true),
        ("traveled", true, false, false),
        ("travelled", true, true, true),
        ("catalog", true, false, false),
        ("catalogue", true, true, true),
        ("anaemia", false, true, true),
        ("anemia", true, true, false),
        ("tire", true, true, true),
        ("tyre", false, false, true),
        ("programme", false, true, true),
        ("program", true, true, true),
        ("meter", true, true, true),
        ("metre", false, true, true),
        ("plough", false, true, true),
        ("colour-blind", false, true, true),
        // Words spelled alike everywhere, and real misspellings, are unaffected.
        ("humorous", true, true, true),
        ("honorary", true, true, true),
        ("tour", true, true, true),
        ("four", true, true, true),
        ("programmed", true, true, true),
        ("colourr", false, false, false),
        ("colorr", false, false, false),
        ("organisationz", false, false, false),
    ];
    for (w, in_us, in_ca, in_gb) in cases {
        assert_eq!(is_correct_in(w, us), in_us, "en-US: {w}");
        assert_eq!(is_correct_in(w, ca), in_ca, "en-CA: {w}");
        assert_eq!(is_correct_in(w, gb), in_gb, "en-GB: {w}");
        assert_eq!(is_correct_for(w, Some("en-AU")), in_gb, "en-AU follows British spelling: {w}");
    }
    // The dialect's own spelling comes first, in the word's case.
    assert_eq!(suggest_in("colour", 5, us).first().map(String::as_str), Some("color"));
    assert_eq!(suggest_in("Colours", 5, us).first().map(String::as_str), Some("Colors"));
    assert_eq!(suggest_in("color", 5, gb).first().map(String::as_str), Some("colour"));
    assert_eq!(suggest_in("organisation", 5, us).first().map(String::as_str), Some("organization"));
    assert_eq!(suggest_in("organisation", 5, ca).first().map(String::as_str), Some("organization"));
    assert_eq!(suggest_in("analyze", 5, gb).first().map(String::as_str), Some("analyse"));
    assert_eq!(suggest_in("CENTER", 5, gb).first().map(String::as_str), Some("CENTRE"));
    assert_eq!(suggest_in("colour's", 5, us).first().map(String::as_str), Some("color's"));
    assert_eq!(suggest_in("colour-blind", 5, us).first().map(String::as_str), Some("color-blind"));
    // Shared suggestions come in the dialect's spelling, never the other one's.
    let gb_sugg = suggest_in("colouur", 8, gb);
    assert!(gb_sugg.contains(&"colour".to_string()) && !gb_sugg.contains(&"color".to_string()), "{gb_sugg:?}");
    let us_sugg = suggest_in("colouur", 8, us);
    assert!(us_sugg.contains(&"color".to_string()) && !us_sugg.contains(&"colour".to_string()), "{us_sugg:?}");
    // A word in the user dictionary is right in every English.
    add_word("zzcolourzz");
    assert!(is_correct_in("zzcolourzz", us));
    // Issues carry the dialect, so suggestions on demand come from the same one.
    let issues = check_text("The color is nice.", &[(0..18, gb)]);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!((issues[0].lang, &"The color is nice."[issues[0].start..issues[0].end]), (gb, "color"));
    let grammar = check_text("the colour is nice.", &[(0..19, gb)]);
    assert!(grammar.iter().any(|i| i.kind == IssueKind::Grammar && i.lang == gb), "{grammar:?}");
}

#[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
#[test]
fn address_token_covers_every_word_split_out_of_it() {
    // English results depend on the dictionaries other tests install.
    crate::hold_dictionary_tests();
    // #1335: one whitespace-delimited token, however punctuation splits it, has one
    // web/e-mail classification, shared by all its words.
    let t = "zxq@mail.example.com,blorp;qwirpy wugwug";
    let bad: Vec<&str> = check_spelling(t).iter().map(|i| &t[i.start..i.end]).collect();
    assert_eq!(bad, vec!["wugwug"]);
    let opts = Options { ignore_internet: false, ..Options::default() };
    let bad: Vec<&str> = check_spelling_with(t, &opts).iter().map(|i| &t[i.start..i.end]).collect();
    assert_eq!(bad.first(), Some(&"zxq"), "{bad:?}");
    assert!(bad.contains(&"qwirpy"), "{bad:?}");
    assert_eq!(bad.last(), Some(&"wugwug"), "{bad:?}");
}

#[test]
fn ellipses_do_not_start_sentences() {
    for text in [
        "I saw... a figure right outside.",
        "I saw… a figure right outside.",
        "I saw.... a figure right outside.",
        "I saw... \"a figure\" right outside.",
        "I saw… “a figure” right outside.",
    ] {
        let capitals: Vec<_> = check_grammar(text).into_iter().filter(|issue| issue.message == "Capitalize the first word of a sentence").collect();
        assert!(capitals.is_empty(), "{text}: {capitals:?}");
    }

    for text in ["I saw. a figure.", "I saw! a figure.", "I saw? a figure.", "I saw.\" a figure."] {
        let capitals: Vec<_> = check_grammar(text).into_iter().filter(|issue| issue.message == "Capitalize the first word of a sentence").collect();
        assert_eq!(capitals.len(), 1, "{text}: {capitals:?}");
        assert_eq!(capitals[0].suggestions, ["A"], "{text}");
    }
}

/// #40: saving merges in words another process saved, and leaves out the ones just removed.
#[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
#[test]
fn saving_merges_with_the_file() {
    // English results depend on the dictionaries other tests install.
    crate::hold_dictionary_tests();
    let dir = std::env::temp_dir().join(format!("wc-dict-{}-{}", std::process::id(), "merge"));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("custom-dictionary.txt");
    std::fs::write(&path, "zxqmergeother\nzxqmergegone\n").unwrap();
    add_word("zxqmergemine");
    save_user_dictionary(&path, &["zxqmergegone".to_string()]).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.lines().any(|w| w == "zxqmergeother"), "the other process's word is kept: {saved}");
    assert!(saved.lines().any(|w| w == "zxqmergemine"), "{saved}");
    assert!(!saved.lines().any(|w| w == "zxqmergegone"), "a removed word stays out: {saved}");
    assert!(is_correct("zxqmergeother") && !is_correct("zxqmergegone"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #3562 review: Add to Dictionary, Ignore All, removal and the dictionary file use the same
/// composed key as spelling, so a word added or ignored decomposed is accepted in both forms,
/// an NFD entry in a saved file loads (and saves) composed, and either form removes it.
#[test]
fn user_dictionary_and_ignore_all_match_both_normalization_forms() {
    crate::hold_dictionary_tests();
    es::test_dict::install();
    let es = ProofLang::Es;
    let (nfd, nfc) = ("zqprue\u{301}ba", "zqpruéba");
    let text = format!("esto {nfd}");
    let issues = check_text(&text, &[(0..text.len(), es)]);
    assert_eq!(issues.iter().map(|i| (i.start, i.end)).collect::<Vec<_>>(), [(5, text.len())], "flagged at the typed bytes");
    assert!(!is_correct_in(nfc, es));
    add_word(nfd);
    assert!(is_correct_in(nfd, es) && is_correct_in(nfc, es));
    assert!(check_text(&text, &[(0..text.len(), es)]).is_empty());
    assert!(user_dictionary().contains(&nfc.to_string()) && !user_dictionary().contains(&nfd.to_string()));
    assert!(remove_word(nfc), "the composed form removes the entry added decomposed");
    assert!(!is_correct_in(nfd, es) && !is_correct_in(nfc, es));
    // Ignore All, either way round.
    ignore_word("zqigno\u{301}ro");
    assert!(is_correct_in("zqignóro", es) && is_correct_in("zqigno\u{301}ro", es));
    ignore_word("zqcallé");
    assert!(is_correct_in("zqcalle\u{301}", es));
    // A dictionary file with a decomposed entry loads composed and is saved composed.
    let path = std::env::temp_dir().join(format!("wc-nfd-dictionary-{}.txt", std::process::id()));
    std::fs::write(&path, "zqguarda\u{301}do\nzqotra\n").unwrap();
    let added = load_user_dictionary(&path);
    assert!(added.contains(&"zqguardádo".to_string()) && !added.iter().any(|w| w.contains('\u{301}')), "{added:?}");
    assert!(is_correct_in("zqguardádo", es) && is_correct_in("zqguarda\u{301}do", es));
    save_user_dictionary(&path, &["zqotra".to_string()]).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.lines().any(|l| l == "zqguardádo") && !saved.contains('\u{301}') && !saved.lines().any(|l| l == "zqotra"), "{saved}");
    // Removing the decomposed form takes out the composed entry, and the file follows.
    assert!(remove_word("zqguarda\u{301}do"));
    save_user_dictionary(&path, &["zqguarda\u{301}do".to_string()]).unwrap();
    assert!(!std::fs::read_to_string(&path).unwrap().lines().any(|l| l == "zqguardádo"));
    assert!(!is_correct_in("zqguardádo", es));
    let _ = std::fs::remove_file(&path);
}

fn flagged(t: &str, v: &[Issue]) -> Vec<String> {
    v.iter().filter(|i| i.kind == IssueKind::Spelling).map(|i| t[i.start..i.end].to_string()).collect()
}

#[cfg_attr(not(craft_proof_data), ignore = "needs the Language data (CRAFT_I18N_DIR)")]
#[test]
fn mixed_language_paragraphs_check_each_run_in_its_language() {
    // English results depend on the dictionaries other tests install.
    crate::hold_dictionary_tests();
    pl::test_dict::install();
    // A Polish paragraph quoting English: English spelling and rules inside the quote only.
    let t = "Napisał „the quick brown fox is is here” i poszedł spać, bo żeka wylała.";
    let (a, b) = (t.find("the").unwrap(), t.find('”').unwrap());
    let spans = [(0..a, ProofLang::Pl), (a..b, ProofLang::En), (b..t.len(), ProofLang::Pl)];
    let v = check_text(t, &spans);
    assert_eq!(flagged(t, &v), ["żeka"], "{v:?}");
    let rep = v.iter().find(|i| i.kind == IssueKind::Grammar).unwrap();
    assert_eq!((rep.lang, rep.message.as_str()), (ProofLang::En, "Repeated word: \"is\""), "{v:?}");
    // The English rules don't run over the Polish text (no "Capitalize" for the lower-case
    // Polish words), and the Polish rules don't run over the English quote.
    assert!(v.iter().all(|i| i.lang == ProofLang::Pl || (a..b).contains(&i.start)), "{v:?}");
    // Text outside every span is English; no spans at all is English.
    assert_eq!(flagged("helo świat", &check_text("helo świat", &[])), ["helo", "świat"]);
    assert_eq!(flagged("helo świat", &check_text("helo świat", &[(5..12, ProofLang::Pl)])), ["helo"]);
    // A language without tools: nothing flagged there.
    assert!(check_text("Bonjour mes amis", &[(0..16, ProofLang::Other)]).is_empty());
    // Portuguese checks in their span only, with the Portuguese message.
    pt::test_dict::install();
    let t = "Ele disse que coraçao e frequencia existem.";
    let v = check_text(t, &[(0..t.len(), ProofLang::Pt)]);
    assert_eq!(flagged(t, &v), ["coraçao", "frequencia"], "{v:?}");
    assert!(v.iter().all(|i| i.lang == ProofLang::Pt && i.message == "Possível erro de ortografia"), "{v:?}");
    // Italian checks in its span only, with the Italian message.
    it::test_dict::install();
    let t = "Vado a Roma con lbiro e caffe.";
    let v = check_text(t, &[(0..t.len(), ProofLang::It)]);
    assert_eq!(flagged(t, &v), ["lbiro", "caffe"], "{v:?}");
    assert!(v.iter().all(|i| i.lang == ProofLang::It && i.message == "Possibile errore di ortografia"), "{v:?}");
}

proptest::proptest! {
    #[test]
    fn mixed_spans_never_panic(s in "\\PC{0,60}", cuts in proptest::collection::vec((0usize..80, 0usize..80, 0u8..15), 0..6)) {
        let langs = [ProofLang::En, ProofLang::EnGb, ProofLang::EnCa, ProofLang::Pl, ProofLang::Pt, ProofLang::It, ProofLang::Tr, ProofLang::Ru, ProofLang::Cs, ProofLang::Id, ProofLang::Es, ProofLang::EsMx, ProofLang::Es419, ProofLang::EsUy, ProofLang::Other];
        let spans: Vec<(Range<usize>, ProofLang)> = cuts.iter().map(|&(a, b, l)| (a.min(b)..a.max(b), langs[l as usize % langs.len()])).collect();
        for i in check_text(&s, &spans) {
            proptest::prop_assert!(i.start <= i.end && i.end <= s.len());
        }
    }
}
