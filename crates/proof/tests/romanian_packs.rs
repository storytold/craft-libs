//! The Romanian specialization retains 0.15's pack lookup, reload and script isolation.
use craft_proof::{ProofLang, hyphen, packs, ro, set_dictionaries_folder};

fn installed_path() -> Option<std::path::PathBuf> {
    craft_proof::installed_dictionaries().into_iter().find(|(lang, _)| *lang == ProofLang::Ro).and_then(|(_, path)| path)
}

#[test]
fn romanian_pack_spelling_hyphenation_and_folder_precedence() {
    // This integration test owns its process; no shared test dictionary or real documents.
    let root = std::env::temp_dir().join(format!("craft-ro-pack-{}", std::process::id()));
    // A folder left by an earlier run with the same process id would be found first.
    let _ = std::fs::remove_dir_all(&root);
    let pack = root.join("packs/ro-ro");
    let folder = root.join("dictionaries");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(pack.join("pack.toml"), "locale = \"ro-ro\"\nlicence = \"MIT OR Apache-2.0\"\n").unwrap();
    std::fs::write(pack.join("spelling.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(pack.join("spelling.dic"), "2\nștiință\nsintetic\n").unwrap();
    std::fs::write(pack.join("hyphenation.pat"), "s2i n2t e2t i2c\n\\hyphenation{sin-te-tic}\n").unwrap();
    set_dictionaries_folder(Some(folder.clone()));
    packs::set_packs_dir(Some(root.join("packs")));
    assert_eq!(installed_path(), Some(pack.join("spelling.dic")));
    assert!(ro::is_correct("ştiinţă"));
    let lim = hyphen::Limits::default();
    assert_eq!(hyphen::hyphenate_word_in("sintetic", &lim, ProofLang::Ro), "sin-te-tic");
    for tag in ["ro", "ro-RO", "ro-MD", "ro-Latn", "ron", "rum", "mo", "mol"] {
        assert_eq!(ProofLang::from_tag(Some(tag)), ProofLang::Ro, "{tag}");
    }
    for tag in ["ro-Cyrl", "mo-Cyrl"] {
        assert!(ProofLang::from_tag(Some(tag)).pack_locale().is_some(), "{tag}");
    }
    std::fs::write(folder.join("ro_RO.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(folder.join("ro_RO.dic"), "1\nexemplusintetic\n").unwrap();
    set_dictionaries_folder(Some(folder.clone()));
    assert_eq!(installed_path(), Some(folder.join("ro_RO.dic")));
    assert!(ro::is_correct("exemplusintetic") && !ro::is_correct("sintetic"));
    assert_eq!(hyphen::hyphenate_word_in("sintetic", &lim, ProofLang::Ro), "sin-te-tic");
    ro::install_dictionary("", "1\ntextmemorie\n").unwrap();
    assert_eq!(installed_path(), None);
    assert!(ro::is_correct("textmemorie"));
    assert!(ro::install_dictionary("FLAG bogus\n", "1\ninvalid\n").is_err());
    assert!(ro::is_correct("textmemorie"));
    set_dictionaries_folder(Some(folder));
    assert!(ro::is_correct("exemplusintetic"));
    set_dictionaries_folder(None);
    let _ = std::fs::remove_dir_all(&root);
}
