//! The command search: how `SearchIndex` and `rank` find and order commands, every word of a query and the command id.

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
fn words_only_descriptions_hold_leave_room_for_the_label_word_a_typo_stands_for() {
    // 50 description words are 1 edit from the query and the label word is 2 edits from it; only words of labels and
    // paths are counted against the typo limit.
    let last = ('a'..='z').filter(|&c| c != 'e').map(|c| format!("rectangl{c}"));
    let first = ('a'..='z').filter(|&c| c != 'r').map(|c| format!("{c}ectangle"));
    let near: Vec<String> = last.chain(first).collect();
    let entries = vec![entry("t0", "Zip").with_words(near.join(" ")), entry("t1", "Rectanqqe")];
    assert_eq!(find(entries, "rectangle"), vec![1]);
}

/// Entries 0, 3 and 5 start with `word`; 1 and 4 hold it after a shorter word, so their labels are shorter; 2 does not
/// hold it. The word's entries are first, between and last among those that hold it.
fn starts_and_holds(word: &str) -> Vec<HelpEntry> {
    let labels = [format!("{word} Qqq"), format!("Q {word}"), "Other".to_owned(), format!("{word} Www"), format!("W {word}"), format!("{word} Zzz")];
    labels.iter().enumerate().map(|(at, label)| entry(&format!("t{at}"), label)).collect()
}

/// Asserts that every entry of `starts` and `holds` is in `found` and that each entry of `starts` ranks above each entry
/// of `holds`.
fn starts_rank_first(found: &[usize], starts: &[usize], holds: &[usize], what: &str) {
    let place = |e: usize| found.iter().position(|&f| f == e);
    for s in starts {
        for h in holds {
            let (Some(s_at), Some(h_at)) = (place(*s), place(*h)) else {
                panic!("{what}: entries {s} and {h} are both found in {found:?}");
            };
            assert!(s_at < h_at, "{what}: entry {s} ranks above entry {h} in {found:?}");
        }
    }
}

#[trial]
fn a_typo_or_a_synonym_is_read_as_the_word_each_entry_holds() {
    // The query is read with the word each entry holds, so an entry it starts ranks above one with a shorter label
    // that holds it later, wherever the entry is among those that hold the word.
    let found = find(starts_and_holds("Shadow"), "shdaow");
    starts_rank_first(&found, &[0, 3, 5], &[1, 4], "a typo");
    assert!(!found.contains(&2), "an entry without the word is not found through a typo");
    let found = find(starts_and_holds("Delete"), "erase");
    starts_rank_first(&found, &[0, 3, 5], &[1, 4], "a synonym");
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

/// An app's tie order (a tool before an effect) decides between entries that match as well, before the label's length,
/// and never lifts a worse match.
#[trial]
fn a_tie_order_puts_an_equal_match_first_but_never_past_a_better_one() {
    let entries =
        vec![entry("t0", "Rect Shape").with_tie_order(3), entry("t1", "Rect Tool Long").with_tie_order(0), entry("t2", "Rect").with_tie_order(5)];
    // The exact label first whatever its order, then the lower order though its label is longer.
    assert_eq!(find(entries, "rect"), vec![2, 1, 0]);
}

#[trial]
fn a_recent_place_lifts_an_entry_with_that_path() {
    let entries =
        || vec![entry("t0", "Copy A").with_path(["Edit"]), entry("t1", "Copy B").with_path(["Edit"]), entry("t2", "Copy C").with_path(["Edit"])];
    assert_eq!(ranked(entries(), english(), "copy", Vec::new(), Vec::new()), vec![0, 1, 2]);
    assert_eq!(ranked(entries(), english(), "copy", vec![2, 0], Vec::new()), vec![2, 0, 1]);
}

#[trial]
fn every_word_of_a_query_must_match_in_any_order_and_the_command_id_is_found() {
    let table = || vec![entry("insert.table", "Table").with_path(["Insert", "Tables"])];
    assert_eq!(find(table(), "insert table"), vec![0]);
    assert_eq!(find(table(), "  Table   INSERT "), vec![0], "any order, any case, extra spaces");
    assert!(find(table(), "insert chart").is_empty(), "every word has to match");
    assert!(find(table(), "").is_empty());
    assert!(find(table(), "   ").is_empty());

    let one = vec![entry("t0", "Other").with_path(["Format", "Italic"]), entry("t1", "Italic").with_path(["Format"])];
    assert_eq!(find(one, "italic"), vec![1, 0], "a label match ranks before a path-only match");

    let ids = || vec![entry("format.italic", "Slanted").with_path(["Format"]), entry("tools.x", "Format.italic tool").with_path(["Tools"])];
    let mut found = find(ids(), "FORMAT.ITALIC");
    found.sort_unstable();
    assert_eq!(found, vec![0, 1], "the id matches in any case, as does the label that starts with it");
    assert_eq!(find(ids(), "slanted format.italic"), vec![0], "a word in the label and a word in the id");
}

/// The index built without a `Ui` (a command's search) ranks as the one a frame under the same host builds.
#[trial]
fn an_index_built_for_a_host_ranks_as_the_frame_under_it_does() {
    let host = std::sync::Arc::new(craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).build());
    let entries = vec![
        entry("t0", "Zip").with_words("copy the layer"),
        entry("t1", "Zzz").with_path(["Copy Tools"]),
        entry("t2", "Make Copy"),
        entry("t3", "Outline Stroke").with_path(["Object", "Path"]),
    ];
    let queries = ["copy", "tools", "object path", "outlines"];
    let in_frame: Rc<RefCell<Vec<Vec<usize>>>> = Rc::default();
    let (out, listed) = (in_frame.clone(), entries.clone());
    let mut fixture = Fixture::new(move |ui| {
        let index = SearchIndex::new(&Env::of(ui).view(), &listed, &english());
        *out.borrow_mut() = queries.iter().map(|query| rank(&index, query, &[], &|_| 0)).collect();
    });
    fixture.host(host.clone());
    fixture.settle();
    let index = SearchIndex::for_host(&host, &entries, &english());
    let without_ui: Vec<Vec<usize>> = queries.iter().map(|query| rank(&index, query, &[], &|_| 0)).collect();
    assert!(without_ui.iter().all(|found| !found.is_empty()), "every query finds something: {without_ui:?}");
    assert_eq!(without_ui, in_frame.take());
}
