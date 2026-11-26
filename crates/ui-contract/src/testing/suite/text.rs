//! Text cases: paste, IME composition and the editor's lines in the AccessKit tree.

use super::drive::{Plan, focus_subject, input_fixture};
use super::{ANY, Case, Subject};
use crate::testing::fixture::Fixture;
use crate::testing::story::EditKind;
use craft_behavior::Input;
use egui::{Event, ImeEvent, Key, Modifiers};

pub(super) const CASES: &[Case] = &[
    Case::new("paste-once", ANY, paste_lands_once),
    Case::new("ime", ANY, ime_composition_is_guarded),
    Case::new("editor-lines", ANY, editor_lines_reach_accesskit),
];

fn is_text(subject: &Subject) -> bool {
    matches!(subject.probe.input, Input::Text | Input::Editor)
}

/// The text the focused component shows. Its text runs and value are on the component's node, and egui's focus is
/// on the text part's widget below it, whose own node holds the value only for a text field.
fn shown_text(fixture: &Fixture) -> String {
    let Some(focus) = fixture.focused() else { return String::new() };
    let tree = fixture.tree();
    let from = |id| {
        let runs = tree.run_texts(id).join("\n");
        match tree.value(id) {
            Some(value) if runs.is_empty() => value.to_owned(),
            _ => runs,
        }
    };
    let text = from(focus);
    match fixture.root_id() {
        Some(root) if text.is_empty() => from(root),
        _ => text,
    }
}

/// The stories whose drive types into the component, focused and editable.
fn typing_fixtures(subject: &Subject) -> Vec<(String, Fixture)> {
    let mut found = Vec::new();
    for story in &subject.stories {
        if !matches!(subject.base(story).plan, Plan::Type(..)) {
            continue;
        }
        let mut fixture = input_fixture(story);
        let editable = focus_subject(&mut fixture)
            && fixture
                .tree()
                .keys()
                .into_iter()
                .any(|key| fixture.tree().data_of(key).is_some_and(|node| !node.is_read_only() && !node.is_disabled()));
        if editable {
            found.push((story.name.to_owned(), fixture));
        }
    }
    found
}

/// A paste event lands once in the focused text part.
fn paste_lands_once(subject: &Subject) -> Result<(), String> {
    if !is_text(subject) {
        return Ok(());
    }
    for (name, mut fixture) in typing_fixtures(subject) {
        fixture.frame(vec![Event::Paste("qzq".to_owned())]);
        fixture.settle();
        let count = shown_text(&fixture).matches("qzq").count();
        if count != 1 {
            return Err(format!("{name}: a paste landed {count} times"));
        }
    }
    Ok(())
}

/// Preedit then commit gives the committed text once; Enter and Escape while composing neither submit nor cancel;
/// the caret rect is reported.
fn ime_composition_is_guarded(subject: &Subject) -> Result<(), String> {
    if !is_text(subject) {
        return Ok(());
    }
    for (name, mut fixture) in typing_fixtures(subject) {
        fixture.frame(vec![Event::Ime(ImeEvent::Preedit { text: "kzk".to_owned(), active_range_chars: None })]);
        match fixture.ime() {
            Some(ime) if ime.caret.min.is_finite() && ime.caret.max.is_finite() => {}
            _ => return Err(format!("{name}: no caret rect was reported while composing")),
        }
        fixture.take_outputs();
        fixture.key(Key::Enter, Modifiers::NONE);
        fixture.key(Key::Escape, Modifiers::NONE);
        let kinds: Vec<EditKind> = fixture.take_outputs().into_iter().flat_map(|output| output.edits).collect();
        if kinds.iter().any(|kind| matches!(kind, EditKind::Commit | EditKind::Set | EditKind::Cancel)) {
            return Err(format!("{name}: Enter or Escape while composing gave {kinds:?}"));
        }
        fixture.frame(vec![Event::Ime(ImeEvent::Commit("kzk".to_owned()))]);
        fixture.settle();
        let count = shown_text(&fixture).matches("kzk").count();
        if count != 1 {
            return Err(format!("{name}: a composition landed {count} times"));
        }
    }
    Ok(())
}

/// An editor that paints typed text reports it as AccessKit text runs. At rest an editor can hold no text (a sheet
/// shows its cells and opens the editor on the first key), so the case types first.
fn editor_lines_reach_accesskit(subject: &Subject) -> Result<(), String> {
    if !matches!(subject.probe.input, Input::Editor) {
        return Ok(());
    }
    for (name, mut fixture) in typing_fixtures(subject) {
        fixture.frame(vec![Event::Text("qzq".to_owned())]);
        fixture.settle();
        if !fixture.texts().iter().any(|run| run.text.contains("qzq")) {
            continue;
        }
        let tree = fixture.tree();
        let reported = tree
            .keys()
            .into_iter()
            .filter_map(|key| tree.data_of(key))
            .any(|node| node.role() == crate::runtime::egui::access::Role::TextRun && node.value().is_some_and(|value| value.contains("qzq")));
        if !reported {
            return Err(format!("{name}: the editor paints the typed text but reports no text run for it"));
        }
    }
    Ok(())
}
