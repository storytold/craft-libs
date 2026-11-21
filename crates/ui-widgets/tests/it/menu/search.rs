//! The command search: how `SearchIndex` and `rank` find and order commands.

use craft_i18n::Words;
use craft_ui::menu::{HelpEntry, SearchIndex, SearchWords, Synonyms, rank};
use craft_ui::runtime::commands::CommandRef;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Fixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

fn entry(id: &str, label: &str) -> HelpEntry {
    HelpEntry::new(CommandRef::new(id), label.to_owned())
}

fn english() -> Synonyms {
    Synonyms::of(&Words::english().set::<SearchWords>())
}

/// The positions `rank` lists for `query`, built from the environment a frame gives a component.
fn ranked(entries: Vec<HelpEntry>, synonyms: Synonyms, query: &str, recent: Vec<usize>, uses: Vec<u32>) -> Vec<usize> {
    let found: Rc<RefCell<Vec<usize>>> = Rc::default();
    let out = found.clone();
    let query = query.to_owned();
    let mut fixture = Fixture::new(move |ui| {
        let env = Env::of(ui);
        let index = SearchIndex::new(&env.view(), &entries, &synonyms);
        *out.borrow_mut() = rank(&index, &query, &recent, &|at| uses.get(at).copied().unwrap_or(0));
    });
    fixture.settle();
    found.take()
}

fn find(entries: Vec<HelpEntry>, query: &str) -> Vec<usize> {
    ranked(entries, english(), query, Vec::new(), Vec::new())
}

#[trial]
fn a_label_ranks_before_its_path_and_a_command_found_only_by_its_description() {
    let entries = vec![
        entry("t0", "Zip").with_words("copy the layer"),
        entry("t1", "Zzz").with_path(["Copy Tools"]),
        entry("t2", "Make Copy"),
        entry("t3", "Copy Merged"),
        entry("t4", "Copy"),
        entry("t5", "Clone It").with_ids(["copy"]),
    ];
    // Exact id, exact label, label prefix, label word, path word, description words only.
    assert_eq!(find(entries, "copy"), vec![5, 4, 3, 2, 1, 0]);
}

#[trial]
fn stems_typos_and_a_substring_find_what_the_words_do_not() {
    let entries = || vec![entry("t0", "Outline Stroke"), entry("t1", "Rectangle"), entry("t2", "Cat")];
    assert_eq!(find(entries(), "outlines"), vec![0], "a plural finds the singular");
    assert_eq!(find(entries(), "rectngle"), vec![1], "a typo of a long word finds it");
    assert_eq!(find(entries(), "tangl"), vec![1], "text inside a word finds it");
    assert!(find(entries(), "cot").is_empty(), "a typo of a short word finds nothing");
}

#[trial]
fn a_synonym_group_finds_its_members_and_a_translated_row_adds_more() {
    let entries = || vec![entry("t0", "Delete Layer"), entry("t1", "Effacer Tout"), entry("t2", "Phrase")];
    assert_eq!(find(entries(), "erase"), vec![0], "a word of a group finds a command labelled with another");
    assert!(!find(entries(), "effacer").contains(&0), "without a translation the French word is no member");
    let row = "remove, delete, erase, clear, effacer, une phrase trop longue, anticonstitutionnellement";
    let translated = Words::new(move |word| (word.english == "remove, delete, erase, clear").then(|| row.to_owned()));
    let synonyms = || Synonyms::of(&translated.set::<SearchWords>());
    assert!(ranked(entries(), synonyms(), "effacer", Vec::new(), Vec::new()).contains(&0), "a translated single word joins the group");
    assert!(!ranked(entries(), synonyms(), "erase", Vec::new(), Vec::new()).contains(&2), "a phrase of the row is ignored");
}

#[trial]
fn use_counts_lift_a_command_but_never_past_a_better_match() {
    let entries = || vec![entry("t0", "Copy A"), entry("t1", "Copy B"), entry("t2", "Copy C"), entry("t3", "Copy")];
    // Counts 1 and 2 share a grade and keep catalog order; 5000 is capped and tops its tier; the exact label wins
    // with no use.
    let found = ranked(entries(), english(), "copy", Vec::new(), vec![1, 2, 5000, 0]);
    assert_eq!(found, vec![3, 2, 0, 1]);
}

#[trial]
fn a_recent_place_lifts_an_entry_with_that_path() {
    let entries = || vec![entry("t0", "Copy A").with_path(["Edit"]), entry("t1", "Copy B").with_path(["Edit"]), entry("t2", "Copy C").with_path(["Edit"])];
    assert_eq!(ranked(entries(), english(), "copy", Vec::new(), Vec::new()), vec![0, 1, 2]);
    assert_eq!(ranked(entries(), english(), "copy", vec![2, 0], Vec::new()), vec![2, 0, 1]);
}
