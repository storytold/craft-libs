use std::borrow::Cow;
use std::collections::HashSet;

use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

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
    many: Option<Vec<String>>,
    fail_check: bool,
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
        if self.fail_check {
            return Err("the dictionary could not be read".into());
        }
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
        if let Some(many) = self.many.clone().filter(|_| issue.word == "jumpd") {
            return Ok(many);
        }
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
    selected: egui::Color32::DARK_BLUE,
    accent: egui::Color32::BLUE,
    on_accent: egui::Color32::WHITE,
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
fn the_first_suggestion_is_ready_to_apply_and_a_click_chooses_another() {
    let mut h = harness(started(TEXT));
    assert_eq!(h.state().0.replacement, "jumped");
    h.get_by_label("jumps").click();
    h.run_steps(2);
    assert_eq!(h.state().0.replacement, "jumps");
    assert_eq!(h.state().1.edits, 0, "a click only chooses");
    h.get_by_label("Change").click();
    h.run_steps(2);
    assert_eq!(h.state().1.text, "The jumps fox. A jumpd dog. The wrng cat.");
    assert_eq!(h.state().1.edits, 1);
    assert_eq!(h.state().0.current().unwrap().start, 17);
}

#[test]
fn a_double_click_applies_a_suggestion() {
    // Frames 1 ms apart, so two clicks make a double-click.
    let mut h = Harness::builder()
        .with_size(egui::vec2(320.0, 800.0))
        .with_step_dt(0.001)
        .build_ui_state(|ui, (review, doc): &mut State| panel(ui, review, doc, &English, &COLORS), started(TEXT));
    h.run_steps(2);
    let at = h.get_by_label("jumps").rect().center();
    h.hover_at(at);
    h.step();
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    assert_eq!(h.state().1.text, "The jumps fox. A jumpd dog. The wrng cat.");
    assert_eq!(h.state().1.edits, 1);
}

#[test]
fn nothing_to_change_to_disables_change() {
    let mut h = harness(started("An unknownword jumpd."));
    h.state_mut().0.replacement.clear();
    h.run_steps(1);
    assert!(h.get_by_label("Change").accesskit_node().is_disabled());
    h.state_mut().0.replacement = "jumpd".into();
    h.run_steps(1);
    assert!(h.get_by_label("Change").accesskit_node().is_disabled(), "the same word changes nothing");
}

/// A docked panel can be short, and a long list must not push the actions out of reach.
#[test]
fn the_actions_stay_in_view_in_a_short_panel() {
    let (mut review, mut doc) = started(TEXT);
    let many = (0..30).map(|i| format!("jump{i}")).collect();
    doc.many = Some(many);
    review.start(&mut doc);
    let height = 240.0;
    let mut h = Harness::builder()
        .with_size(egui::vec2(320.0, height))
        .build_ui_state(|ui, (review, doc): &mut State| panel(ui, review, doc, &English, &COLORS), (review, doc));
    h.run_steps(2);
    for action in ["Change", "Ignore Once", "Ignore All", "Add to Dictionary"] {
        let bottom = h.get_by_label(action).rect().max.y;
        assert!(bottom <= height, "{action} ends at {bottom}");
    }
    h.get_by_label("Ignore All").click();
    h.run_steps(2);
    assert_eq!(h.state().0.current().unwrap().word, "wrng");
}

#[test]
fn the_actions_keep_their_place_from_one_issue_to_the_next() {
    let mut h = harness(started(TEXT));
    let place = h.get_by_label("Ignore Once").rect();
    h.get_by_label("Ignore All").click();
    h.run_steps(2);
    assert_eq!(h.state().0.current().unwrap().suggestions.len(), 1, "a shorter list than the last");
    assert_eq!(h.get_by_label("Ignore Once").rect(), place);
}

/// No gap above the actions, and a list too long for the room scrolls inside itself.
#[test]
fn the_suggestion_list_fills_the_room_and_scrolls_inside_itself() {
    let height = 420.0;
    let panel_of = |many: usize| {
        let mut doc = Doc::new(TEXT);
        doc.many = Some((0..many).map(|i| format!("jump{i}")).collect());
        let mut review = Review::default();
        review.start(&mut doc);
        let mut h = Harness::builder()
            .with_size(egui::vec2(320.0, height))
            .build_ui_state(|ui, (review, doc): &mut State| panel(ui, review, doc, &English, &COLORS), (review, doc));
        h.run_steps(2);
        h
    };
    let long = panel_of(40);
    let change = long.get_by_label("Change").rect();
    assert!(long.get_by_label("Change to:").rect().min.y >= 0.0, "the field above the list isn't scrolled away");
    let last_row = (0..40).map(|i| long.get_by_label(&format!("jump{i}")).rect()).find(|row| row.max.y >= change.min.y - 12.0);
    assert!(last_row.is_some_and(|row| row.min.y < change.min.y), "rows reach the actions");
    assert!(long.get_by_label("jump39").rect().min.y > height, "the rest is scrolled, not squeezed");
    assert_eq!(panel_of(2).get_by_label("Change").rect(), change, "the actions don't move with the list");
    let mut long = long;
    let field = long.get_by_label("Change to:").rect();
    let row = long.get_by_label("jump0").rect();
    long.hover_at(row.center());
    long.step();
    long.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -60.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    });
    long.run_steps(4);
    assert!(long.get_by_label("jump0").rect().min.y < row.min.y, "the list scrolled");
    assert_eq!(long.get_by_label("Change to:").rect(), field, "and only the list");
    assert_eq!(long.get_by_label("Change").rect(), change);
}

/// Going through issues, a click on Ignore Once must not land on Add to Dictionary.
#[test]
fn a_preferred_form_keeps_ignore_once_in_its_place() {
    let place = harness(started(TEXT)).get_by_label("Ignore Once").rect().min;
    let mut doc = Doc::new(TEXT);
    doc.preferred = true;
    let mut review = Review::default();
    review.start(&mut doc);
    assert_eq!(harness((review, doc)).get_by_label("Ignore Once").rect().min, place);
}

#[test]
fn a_narrow_panel_stacks_the_ignore_buttons() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(110.0, 600.0))
        .build_ui_state(|ui, (review, doc): &mut State| panel(ui, review, doc, &English, &COLORS), started(TEXT));
    h.run_steps(2);
    let (once, all) = (h.get_by_label("Ignore Once").rect(), h.get_by_label("Ignore All").rect());
    assert!(once.max.y <= all.min.y, "{once:?} above {all:?}");
    let wide = harness(started(TEXT));
    let (once, all) = (wide.get_by_label("Ignore Once").rect(), wide.get_by_label("Ignore All").rect());
    assert_eq!(once.min.y, all.min.y);
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
    assert!(h.query_by_label("Change to:").is_some());
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
    assert_eq!(h.query_all_by_label("The text changed. Check again to continue.").count(), 1, "said once");
}

#[test]
fn a_check_that_fails_doesnt_say_the_document_is_clean() {
    let mut doc = Doc::new(TEXT);
    doc.fail_check = true;
    let mut review = Review::default();
    review.start(&mut doc);
    let mut h = harness((review, doc));
    assert!(h.query_by_label("No issues found.").is_none());
    assert!(h.query_by_label("the dictionary could not be read").is_some());
    h.state_mut().1.fail_check = false;
    h.get_by_label("Check Again").click();
    h.run_steps(2);
    assert_eq!(h.state().0.current().unwrap().word, "jumpd");
}

/// A dictionary or language change leaves the text and its version alone: only the failure says so.
#[test]
fn an_issue_a_fresh_check_lost_offers_check_again() {
    let (mut review, mut doc) = started(TEXT);
    doc.known.insert("jumpd".into());
    let _ = review.act(&mut doc, Action::IgnoreOnce);
    let mut h = harness((review, doc));
    assert_eq!(h.query_all_by_label("The text changed. Check again to continue.").count(), 1);
    assert!(h.query_by_label("Ignore Once").is_none());
    h.get_by_label("Check Again").click();
    h.run_steps(2);
    assert_eq!(h.state().0.current().unwrap().word, "wrng");
}

/// A dictionary or language change can add findings without touching the text.
#[test]
fn ignoring_takes_in_what_a_fresh_check_found() {
    let mut doc = Doc::new(TEXT);
    doc.known.insert("wrng".into());
    let mut review = Review::default();
    review.start(&mut doc);
    assert_eq!(review.remaining(), 2);
    doc.known.clear();
    review.act(&mut doc, Action::IgnoreAll).unwrap();
    assert_eq!(review.current().unwrap().word, "wrng");
}

#[test]
fn checking_starts_a_review_that_wasnt_started() {
    let mut doc = Doc::new(TEXT);
    let mut review = Review::default();
    review.act(&mut doc, Action::Check).unwrap();
    assert!(review.started());
    assert_eq!(review.current().unwrap().word, "jumpd");
    assert!(!review.other_document(&doc));
}

#[test]
fn marks_are_the_issues_left_in_unchanged_stories() {
    let (mut review, mut doc) = started(TEXT);
    let words =
        |review: &Review, doc: &Doc| -> Vec<String> { review.marks(doc.document(), |story| doc.version(story)).map(|i| i.word.clone()).collect() };
    assert_eq!(words(&review, &doc), ["jumpd", "jumpd", "wrng"]);
    review.act(&mut doc, Action::IgnoreOnce).unwrap();
    assert_eq!(words(&review, &doc), ["jumpd", "wrng"], "an ignored occurrence isn't marked");
    review.act(&mut doc, Action::IgnoreAll).unwrap();
    assert_eq!(words(&review, &doc), ["wrng"]);
    review.act(&mut doc, Action::IgnoreAll).unwrap();
    assert!(words(&review, &doc).is_empty());
    assert!(review.covers(doc.document(), 7, doc.version(7)), "it still speaks for a story of ignored words");
    assert!(!review.covers(doc.document(), 8, doc.version(8)), "not for one it hasn't seen");
    review.act(&mut doc, Action::Check).unwrap();
    doc.edit(0, "A ");
    assert!(!review.covers(doc.document(), 7, doc.version(7)));
    assert!(words(&review, &doc).is_empty(), "offsets into changed text aren't drawn");
    review.act(&mut doc, Action::Check).unwrap();
    doc.id = 2;
    assert!(words(&review, &doc).is_empty(), "nor into another document");
    assert_eq!(Review::default().marks(None, |_| None).count(), 0);
}

#[test]
fn every_string_is_listed_once() {
    for source in
        ["Spelling", "Preferred Form", "Suggestions", "Ignore Once", "Ignore All", "Add to Dictionary", "{count} left", "Change to:", "Check Again"]
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
