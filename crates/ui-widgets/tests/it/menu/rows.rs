//! Menu rows: the detail line under the label, the row's own tooltip, the sample slot and which shortcut a row shows.

use craft_i18n::Direction;
use craft_ui::buttons::Button;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{CommandSet, Props as _, Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::commands::CommandState;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::Rect;
use std::cell::Cell;
use std::rc::Rc;

/// The texts painted inside `row` other than `label`.
fn other_runs(fixture: &Fixture, row: Rect, label: &str) -> Vec<(String, Rect)> {
    let runs = fixture.texts().iter().filter(|run| row.contains(run.rect.center()) && run.text != label);
    runs.map(|run| (run.text.clone(), run.rect)).collect()
}

/// The painted run `text`.
fn run(fixture: &Fixture, text: &str) -> Option<Rect> {
    fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect)
}

/// Where the label, the sample and the shortcut of a row with all three lie under `direction`, and, for a
/// `sample_only` row: whether its label text is painted, whether its name is in the access tree, and its sample.
struct SampleRows {
    label: Rect,
    sample: Rect,
    shortcut: Rect,
    only_label_painted: bool,
    only_named: bool,
    only_sample: Rect,
}

fn sample_rows(direction: Direction) -> SampleRows {
    let sample = Rc::new(Cell::new(Rect::NOTHING));
    let only = Rc::new(Cell::new(Rect::NOTHING));
    let (seen, seen_only) = (Rc::clone(&sample), Rc::clone(&only));
    let mut fixture = Fixture::new(move |ui| {
        Menu::inline("Swatches").show(ui, |ui, _| {
            let (seen, seen_only) = (Rc::clone(&seen), Rc::clone(&seen_only));
            MenuItem::new("Shade").sample(move |_: &ContentPainterCx, rect: Rect| seen.set(rect)).shortcut("F5").show(ui);
            MenuItem::new("Ocean").sample(move |_: &ContentPainterCx, rect: Rect| seen_only.set(rect)).sample_only().show(ui);
        });
    });
    fixture.direction(direction);
    fixture.settle();
    SampleRows {
        label: run(&fixture, "Shade").expect("the label is painted"),
        sample: sample.get(),
        shortcut: run(&fixture, "F5").expect("the shortcut is painted"),
        only_label_painted: run(&fixture, "Ocean").is_some(),
        only_named: fixture.by_label("Ocean").is_some(),
        only_sample: only.get(),
    }
}

#[trial]
fn a_row_sample_sits_between_the_label_and_the_shortcut_and_mirrors() {
    let ltr = sample_rows(Direction::Ltr);
    assert!(ltr.sample.width() > 0.0, "the sample is painted into a slot: {:?}", ltr.sample);
    assert!(ltr.label.max.x <= ltr.sample.min.x + 0.5, "the sample follows the label: {:?} then {:?}", ltr.label, ltr.sample);
    assert!(ltr.sample.max.x <= ltr.shortcut.min.x + 0.5, "and comes before the shortcut: {:?} then {:?}", ltr.sample, ltr.shortcut);

    let rtl = sample_rows(Direction::Rtl);
    assert!(rtl.sample.width() > 0.0, "the sample is painted under right-to-left too");
    assert!(rtl.sample.max.x <= rtl.label.min.x + 0.5, "right to left, the sample lies left of the label: {:?} {:?}", rtl.sample, rtl.label);
    assert!(rtl.shortcut.max.x <= rtl.sample.min.x + 0.5, "and the shortcut left of the sample: {:?} {:?}", rtl.shortcut, rtl.sample);

    for (rows, direction) in [(&ltr, "left to right"), (&rtl, "right to left")] {
        assert!(!rows.only_label_painted, "a sample-only row paints no label text ({direction})");
        assert!(rows.only_named, "and keeps its label as its name in the access tree ({direction})");
        assert!(rows.only_sample.width() > 0.0, "its sample takes the label's place ({direction})");
    }
}

/// The app commands the shortcut trial's rows bind, each with a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "menuRowsTrial")]
#[non_exhaustive]
enum RowCommand {
    #[command(label = "Find", default = "Cmd+K", modal = "always")]
    Find,
    #[command(label = "Replace", default = "Cmd+J", modal = "always")]
    Replace,
    #[command(label = "Jump", default = "Cmd+L", modal = "always")]
    Jump,
}

#[trial]
fn a_caller_shortcut_wins_over_the_bound_commands_chord_and_hide_shortcut_shows_none() {
    let mut fixture = Fixture::new(|ui| {
        Menu::inline("Search").show(ui, |ui, _| {
            MenuItem::command(RowCommand::Find).show(ui);
            MenuItem::command(RowCommand::Replace).shortcut("X").show(ui);
            MenuItem::command(RowCommand::Jump).hide_shortcut().show(ui);
            MenuItem::new("Plain").show(ui);
        });
    });
    for command in [RowCommand::Find, RowCommand::Replace, RowCommand::Jump] {
        fixture.command_state(command, CommandState::new());
    }
    fixture.settle();
    let row = |fixture: &Fixture, label: &str| fixture.by_label(label).expect("each row is in the access tree");
    let listed = |fixture: &Fixture, label: &str| {
        let node = row(fixture, label).node;
        fixture.tree().shortcut_of(node).map(str::to_owned)
    };

    let find = row(&fixture, "Find");
    assert!(!other_runs(&fixture, find.rect, "Find").is_empty(), "a command row shows its command's chord");
    assert!(listed(&fixture, "Find").is_some(), "and lists it as its shortcut");

    let replace = row(&fixture, "Replace");
    let shown: Vec<String> = other_runs(&fixture, replace.rect, "Replace").into_iter().map(|(text, _)| text).collect();
    assert_eq!(shown, ["X"], "the caller's shortcut text wins over the chord");
    assert_eq!(listed(&fixture, "Replace").as_deref(), Some("X"), "and is the row's listed shortcut");

    let jump = row(&fixture, "Jump");
    assert!(other_runs(&fixture, jump.rect, "Jump").is_empty(), "a hidden shortcut paints none");
    assert_eq!(listed(&fixture, "Jump"), None, "and lists none");

    let plain = row(&fixture, "Plain");
    assert!(other_runs(&fixture, plain.rect, "Plain").is_empty(), "a row with no command and no text shows no shortcut");
    assert_eq!(listed(&fixture, "Plain"), None, "and lists none");
}

#[trial]
fn a_detail_shows_as_a_second_line_under_the_label() {
    let mut fixture = Fixture::new(|ui| {
        Menu::inline("Paper").show(ui, |ui, _| {
            MenuItem::new("Letter").show(ui);
            MenuItem::new("A4").detail("210 × 297 mm").show(ui);
        });
    });
    fixture.settle();
    let plain = fixture.by_label("Letter").expect("the plain row").rect;
    let detailed = fixture.by_label("A4").expect("the row with a detail").rect;
    assert!(detailed.height() > plain.height(), "the row with a detail is a line taller");
    let text = |wanted: &str| fixture.texts().iter().find(|run| run.text == wanted).map(|run| run.rect).expect("the run is painted");
    let (label, detail) = (text("A4"), text("210 × 297 mm"));
    assert!(detail.min.y >= label.max.y - 0.5, "the detail lies under the label");
    assert!((detail.min.x - label.min.x).abs() < 1.0, "the detail starts where the label starts");
}

#[trial]
fn a_row_shows_its_tooltip_when_the_pointer_rests_on_it_in_an_open_menu() {
    let mut fixture = Fixture::new(|ui| {
        let opener = Button::new("Open").show(ui);
        Menu::new("Edit").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            MenuItem::new("Undo").tooltip("Take back the last change".to_owned()).show(ui);
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.click(&opener).settle();
    let row = fixture.by_label("Undo").expect("the menu is open").center();
    fixture.frame(vec![pointer::moved(row)]);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(fixture.texts().iter().any(|run| run.text == "Take back the last change"), "the row's tooltip shows");
}

#[trial]
fn a_letter_no_row_takes_while_a_menu_is_open_does_not_reach_the_text_field_under_it() {
    let text = std::rc::Rc::new(std::cell::RefCell::new("hello".to_owned()));
    let held = text.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut value = held.borrow_mut();
        let field = craft_ui::text::TextField::new("Name", craft_ui::contract::Bind::bound(&mut *value)).own_context_menu().show(ui);
        Menu::context("Field menu", &field.response).show(ui, |ui, _| {
            MenuItem::new("Copy").show(ui);
            MenuItem::new("Paste").show(ui);
        });
    });
    fixture.settle();
    let at = fixture.by_label("Name").expect("the field").center();
    fixture.click_at(at);
    fixture.settle();
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    assert!(fixture.by_label("Copy").is_some(), "the context menu is open over the field");
    fixture.type_text("q");
    fixture.settle();
    assert_eq!(*text.borrow(), "hello", "the letter does not reach the field");
}
