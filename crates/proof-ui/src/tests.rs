use std::borrow::Cow;
use std::collections::HashSet;

use egui_kittest::{Harness, kittest::Queryable};

use super::*;
use crate::review::SAME_REPLACEMENT;

const TEXT: &str = "The jumpd fox. A jumpd dog. The wrng cat.";

/// A one-story document that knows two misspellings.
#[derive(Default)]
struct Doc {
    text: String,
    version: u64,
    id: u64,
    known: HashSet<String>,
    revealed: Vec<(usize, usize)>,
    asked: Vec<String>,
    edits: u32,
    notice: Option<String>,
    preferred: bool,
}

impl Doc {
    fn new(text: &str) -> Self {
        Doc { text: text.into(), id: 1, ..Default::default() }
    }
    fn edit(&mut self, at: usize, insert: &str) {
        self.text.insert_str(at, insert);
        self.version += 1;
    }
}

impl Host for Doc {
    fn check(&mut self) -> Result<Vec<Issue>, String> {
        let mut found = Vec::new();
        for word in ["jumpd", "wrng"] {
            if self.known.contains(word) {
                continue;
            }
            for (start, _) in self.text.match_indices(word) {
                let kind = if self.preferred { IssueKind::Grammar } else { IssueKind::Spelling };
                found.push(Issue { story: 7, start, end: start + word.len(), word: word.into(), kind, ..Default::default() });
            }
        }
        found.sort_by_key(|i| i.start);
        Ok(found)
    }
    fn suggestions(&mut self, issue: &Issue) -> Result<Vec<String>, String> {
        self.asked.push(issue.word.clone());
        Ok(match issue.word.as_str() {
            "jumpd" => vec!["jumped".into(), "jumps".into()],
            "wrng" => vec!["wrong".into()],
            _ => vec![],
        })
    }
    fn text(&self, story: u64) -> Option<&str> {
        (story == 7).then_some(self.text.as_str())
    }
    fn version(&self, story: u64) -> Option<u64> {
        (story == 7).then_some(self.version)
    }
    fn document(&self) -> Option<u64> {
        Some(self.id)
    }
    fn reveal(&mut self, issue: &Issue) {
        self.revealed.push((issue.start, issue.end));
    }
    fn change(&mut self, issue: &Issue, to: &str) -> Result<(), String> {
        self.text.replace_range(issue.start..issue.end, to);
        self.version += 1;
        self.edits += 1;
        Ok(())
    }
    fn add_word(&mut self, word: &str) -> Result<(), String> {
        self.known.insert(word.into());
        Ok(())
    }
    fn notice(&self) -> Option<String> {
        self.notice.clone()
    }
}

struct English;
impl Text for English {
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
        source.into()
    }
}

const COLORS: Colors = Colors {
    text: egui::Color32::WHITE,
    dim: egui::Color32::GRAY,
    field: egui::Color32::BLACK,
    field_border: egui::Color32::GRAY,
    hover: egui::Color32::DARK_GRAY,
    spelling: egui::Color32::RED,
    grammar: egui::Color32::BLUE,
    error: egui::Color32::RED,
};

type State = (Review, Doc);

fn harness(state: State) -> Harness<'static, State> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(320.0, 800.0))
        .build_ui_state(|ui, (review, doc): &mut State| panel(ui, review, doc, &English, &COLORS), state);
    h.run_steps(2);
    h
}

fn started(text: &str) -> State {
    let mut doc = Doc::new(text);
    let mut review = Review::default();
    review.start(&mut doc);
    (review, doc)
}

#[test]
fn a_pass_starts_on_the_first_issue_shows_it_and_asks_only_for_its_suggestions() {
    let (review, doc) = started(TEXT);
    assert_eq!(review.remaining(), 3);
    assert_eq!(review.current().unwrap().suggestions, ["jumped", "jumps"]);
    assert_eq!(doc.revealed, [(4, 9)], "the word is shown in the document");
    assert_eq!(doc.asked, ["jumpd"], "never the rest of a book's suggestions");
    assert_eq!(review.replacement, "jumped");
    assert_eq!((review.before.as_str(), review.after.as_str()), ("The ", " fox. A jumpd dog. The wrng cat."));
}

#[test]
fn the_panel_starts_a_review_from_its_button() {
    let mut h = harness((Review::default(), Doc::new(TEXT)));
    h.get_by_label("Check Spelling").click();
    h.run_steps(2);
    assert_eq!(h.state().0.current().unwrap().word, "jumpd");
}

#[test]
fn clicking_a_suggestion_corrects_the_word_and_moves_on() {
    let mut h = harness(started(TEXT));
    h.get_by_label("jumps").click();
    h.run_steps(2);
    assert_eq!(h.state().1.text, "The jumps fox. A jumpd dog. The wrng cat.");
    assert_eq!(h.state().1.edits, 1);
    assert_eq!(h.state().0.current().unwrap().start, 17);
}

#[test]
fn ignore_once_skips_one_occurrence_and_ignore_all_the_word() {
    let mut h = harness(started(TEXT));
    h.get_by_label("Ignore Once").click();
    h.run_steps(2);
    assert_eq!((h.state().0.remaining(), h.state().0.current().unwrap().start), (2, 17));
    assert_eq!(h.state().0.current().unwrap().suggestions, ["jumped", "jumps"], "the next one gets its suggestions");
    h.get_by_label("Ignore All").click();
    h.run_steps(2);
    assert_eq!(h.state().0.current().unwrap().word, "wrng");
    h.get_by_label("Ignore Once").click();
    h.run_steps(2);
    assert!(h.state().0.current().is_none());
    assert_eq!(h.state().1.edits, 0, "ignoring edits nothing");
    assert!(h.query_by_label("The spelling check is complete. Some issues were ignored.").is_some());
    h.get_by_label("Check Again").click();
    h.run_steps(2);
    assert_eq!(h.state().0.remaining(), 3, "a new pass checks everything again");
}

#[test]
fn add_to_dictionary_clears_every_occurrence_and_a_clean_document_says_so() {
    let mut h = harness(started("The jumpd fox. A jumpd dog."));
    h.get_by_label("Add to Dictionary").click();
    h.run_steps(2);
    assert!(h.state().1.known.contains("jumpd"));
    assert!(h.state().0.current().is_none());
    assert!(h.query_by_label("No issues found.").is_some());
}

#[test]
fn a_preferred_form_cant_join_the_dictionary() {
    let mut doc = Doc::new(TEXT);
    doc.preferred = true;
    let mut review = Review::default();
    review.start(&mut doc);
    let mut h = harness((review, doc));
    assert!(h.query_by_label("Preferred Form").is_some());
    assert!(h.query_by_label("Add to Dictionary").is_none());
    assert!(h.query_by_label("Ignore All").is_none());
    let (review, doc) = h.state_mut();
    assert!(review.act(doc, Action::AddToDictionary).unwrap_err().contains("Only spelling issues"));
}

#[test]
fn a_typed_replacement_is_applied_from_the_change_field() {
    let mut h = harness(started(TEXT));
    assert!(h.query_by_label("Change to:").is_none(), "out of the way until asked for");
    h.get_by_label("Change…").click();
    h.run_steps(2);
    h.state_mut().0.replacement = "typed".into();
    h.run_steps(1);
    h.get_by_label("Change").click();
    h.run_steps(2);
    assert_eq!(h.state().1.text, "The typed fox. A jumpd dog. The wrng cat.");
}

#[test]
fn the_same_or_an_empty_replacement_is_refused() {
    let (mut review, mut doc) = started(TEXT);
    for to in ["", "jumpd"] {
        assert_eq!(review.act(&mut doc, Action::Change(Some(to.into()))).unwrap_err(), SAME_REPLACEMENT);
    }
    assert_eq!(doc.edits, 0);
}

#[test]
fn edited_text_stops_corrections_until_checked_again() {
    let (mut review, mut doc) = started(TEXT);
    doc.edit(0, "A ");
    let before = doc.text.clone();
    assert!(review.stale(&doc));
    assert!(review.act(&mut doc, Action::Change(Some("jumped".into()))).unwrap_err().contains("text changed"));
    assert_eq!(doc.text, before, "stale offsets are never applied");
    let mut h = harness((review, doc));
    assert!(h.query_by_label("jumped").is_none(), "no suggestion is offered for changed text");
    h.get_by_label("Check Again").click();
    h.run_steps(2);
    assert_eq!(h.state().0.current().unwrap().start, 6, "checked again at its new place");
    assert!(!h.state().0.stale(&h.state().1));
}

/// A language or dictionary edit leaves the text alone, and the issue is gone all the same.
#[test]
fn a_correction_needs_the_issue_to_still_be_found() {
    let (mut review, mut doc) = started(TEXT);
    doc.known.insert("jumpd".into());
    assert!(review.act(&mut doc, Action::Change(Some("jumped".into()))).unwrap_err().contains("text changed"));
    assert_eq!(doc.edits, 0);
}

#[test]
fn a_correction_keeps_ignored_occurrences_at_their_new_offsets() {
    // Before the correction: the ignored offsets stand.
    let (mut review, mut doc) = started("jumpd one wrng two jumpd");
    review.act(&mut doc, Action::IgnoreOnce).unwrap();
    review.act(&mut doc, Action::Change(Some("wrong".into()))).unwrap();
    assert_eq!(doc.text, "jumpd one wrong two jumpd");
    assert_eq!(review.current().unwrap().start, 20, "the first stays ignored; the last is next");
    // After the correction: the ignored offset moves with its word. "wrng" is ignored here by
    // the second pass reaching it first, so correct the word before an ignored one.
    let (mut review, mut doc) = started("wrng one jumpd");
    review.act(&mut doc, Action::Change(Some("wrong".into()))).unwrap();
    assert_eq!(review.current().unwrap().start, 10);
    review.act(&mut doc, Action::IgnoreOnce).unwrap();
    assert!(review.current().is_none());
    assert!(review.ignored_any());
}

#[test]
fn someone_elses_edit_forgets_what_was_ignored_in_that_story() {
    let (mut review, mut doc) = started(TEXT);
    review.act(&mut doc, Action::IgnoreOnce).unwrap();
    doc.edit(0, "A ");
    review.act(&mut doc, Action::Check).unwrap();
    assert_eq!(review.remaining(), 3);
    assert!(!review.ignored_any());
}

#[test]
fn another_document_ends_the_review() {
    let (mut review, mut doc) = started(TEXT);
    doc.id = 2;
    assert!(review.act(&mut doc, Action::IgnoreOnce).unwrap_err().contains("document changed"));
    let mut h = harness((review, doc));
    assert!(h.query_by_label("jumped").is_none());
    h.get_by_label("Check This Document").click();
    h.run_steps(2);
    assert!(!h.state().0.other_document(&h.state().1));
    assert_eq!(h.state().0.current().unwrap().word, "jumpd");
}

#[test]
fn a_notice_and_a_failure_are_shown() {
    let mut doc = Doc::new(TEXT);
    doc.notice = Some("Khmer text has not been checked.".into());
    let mut review = Review::default();
    review.start(&mut doc);
    doc.edit(0, "A ");
    let _ = review.act(&mut doc, Action::Change(Some("jumped".into())));
    let h = harness((review, doc));
    assert!(h.query_by_label("Khmer text has not been checked.").is_some());
    assert_eq!(h.query_all_by_label("The text changed. Check again to continue.").count(), 2, "the state and the failure");
}

#[test]
fn every_string_is_listed_once() {
    for source in
        ["Spelling", "Preferred Form", "Suggestions", "Ignore Once", "Ignore All", "Add to Dictionary", "Change…", "Change to:", "Check Again"]
    {
        assert!(STRINGS.contains(&source), "{source}");
    }
    let mut unique = STRINGS.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), STRINGS.len());
}

fn context_harness(issue: Issue) -> Harness<'static, (Issue, Option<ContextChoice>)> {
    let mut h = Harness::builder().with_size(egui::vec2(320.0, 400.0)).build_ui_state(
        |ui, (issue, choice): &mut (Issue, Option<ContextChoice>)| {
            if let Some(chosen) = context_section(ui, issue, &English) {
                *choice = Some(chosen);
            }
        },
        (issue, None),
    );
    h.run_steps(1);
    h
}

#[test]
fn the_right_click_section_offers_each_choice() {
    let issue = Issue { word: "jumpd".into(), suggestions: vec!["jumped".into()], ..Default::default() };
    for (button, expected) in [
        ("jumped", ContextChoice::Change("jumped".into())),
        ("Add to Dictionary", ContextChoice::AddToDictionary),
        ("Check Spelling…", ContextChoice::Review),
    ] {
        let mut h = context_harness(issue.clone());
        h.get_by_label(button).click();
        h.run_steps(2);
        assert_eq!(h.state().1.as_ref(), Some(&expected));
    }
    let preferred = Issue { kind: IssueKind::Grammar, ..issue };
    assert!(context_harness(preferred).query_by_label("Add to Dictionary").is_none());
    let none = Issue { word: "x".into(), ..Default::default() };
    assert!(context_harness(none).query_by_label("(no suggestions)").is_some());
}

#[test]
fn context_is_bounded_and_stays_in_the_paragraph() {
    let long = format!("{}\n{} jumpd {}\nnext", "x".repeat(10), "b".repeat(200), "a".repeat(200));
    let (review, _) = started(&long);
    assert!(review.before.starts_with('…') && !review.before.contains('x'));
    assert!(review.after.ends_with('…') && !review.after.contains("next"));
    assert!(review.before.chars().count() <= 61 && review.after.chars().count() <= 61);
}
