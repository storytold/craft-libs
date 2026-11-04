//! Local compatibility probe for the pinned hu Language pack. No bundled dictionary.
use std::{error::Error, path::PathBuf};
fn main() -> Result<(), Box<dyn Error>> {
    let packs = PathBuf::from(std::env::var("CRAFT_HU_TEST_PACKS")?);
    let start = std::time::Instant::now();
    let d = craft_proof::hunspell::load(&packs.join("hu/spelling.aff"), &packs.join("hu/spelling.dic"))?;
    println!("LOAD_MS\t{}", start.elapsed().as_millis());
    craft_proof::packs::set_packs_dir(Some(packs));
    let lang = craft_proof::ProofLang::from_tag(Some("hu"));
    let opts = craft_proof::Options { ignore_uppercase: false, ignore_numbers: false, ignore_internet: false };
    let mut total = 0;
    let mut raw = 0;
    let mut api = 0;
    for corpus in
        [include_str!("../tests/data/hu/main.tsv"), include_str!("../tests/data/hu/focused.tsv"), include_str!("../tests/data/hu/extended.tsv")]
    {
        for line in corpus.lines() {
            let (w, want) = line.split_once('\t').ok_or("invalid fixture")?;
            let expected = want == "1";
            total += 1;
            let actual = d.check(w);
            if actual != expected {
                raw += 1;
                println!("SPELL_RAW\t{w}\t{expected}\t{actual}");
            }
            let actual = craft_proof::is_correct_in_with(w, lang, &opts);
            if actual != expected {
                api += 1;
                println!("SPELL_API\t{w}\t{expected}\t{actual}");
            }
        }
    }
    println!("SPELL_TOTAL\t{total}\t{raw}\t{api}");
    for line in include_str!("../tests/data/hu/suggestions.tsv").lines() {
        let (w, want) = line.split_once('\t').ok_or("invalid suggestion fixture")?;
        let start = std::time::Instant::now();
        let got = craft_proof::suggest_in(w, 8, lang);
        println!("SUGGEST\t{w}\t{want}\t{}\t{}\t{:?}", got.iter().any(|x| x == want), start.elapsed().as_millis(), got);
    }
    let lim = craft_proof::hyphen::Limits { min_word: 2, after_first: 1, before_last: 1, capitalized: true };
    for w in ["asszony", "könnyen", "hosszú", "meggyel", "meggyőző", "jegygyűrű"] {
        println!("HYPHEN\t{w}\t{}", craft_proof::hyphen::hyphenate_word_in(w, &lim, lang));
    }
    let mut total = 0;
    let mut mismatches = 0;
    for line in include_str!("../tests/data/hu/hyphen-native.tsv").lines() {
        let (w, want) = line.split_once('\t').ok_or("invalid hyphen fixture")?;
        total += 1;
        let got =
            craft_proof::hyphen::hyphenate_word_in(w, &craft_proof::hyphen::Limits { min_word: 2, ..Default::default() }, lang).replace('-', "=");
        if got != want {
            mismatches += 1;
            println!("HYPHEN_DIFF\t{w}\t{want}\t{got}");
        }
    }
    println!("HYPHEN_TOTAL\t{total}\t{mismatches}");
    Ok(())
}
