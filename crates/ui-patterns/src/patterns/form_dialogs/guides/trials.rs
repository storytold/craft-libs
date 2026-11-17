//! Trials of the guide dialogs.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::{GUIDE_LAYOUT_COLOR_KEY, GuideLayoutDialog, GuideLayoutValues, GuideRect, GuideUnits, NewGuideDialog, NewGuideValues};
use crate::canvas_data::{Guide, GuideColor};
use crate::contract::{Bind, SaveKey};
use crate::runtime::env::UnitKind;
use crate::runtime::requests::ColorRequest;
use crate::testing::relations::{aligned_left, grows_with};
use crate::testing::{Fixture, trial};
use craft_behavior::Axis;
use craft_theme::Rgba;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The owner-less Preview box starts on at every open: turned off, it stays off while the dialog is open only.
#[trial]
fn the_preview_check_box_under_the_side_buttons_starts_on_at_every_open() {
    let open = Rc::new(Cell::new(true));
    let previewing = Rc::new(Cell::new(false));
    let (opened, seen) = (Rc::clone(&open), Rc::clone(&previewing));
    let mut fixture = Fixture::new(move |ui| {
        let units = GuideUnits::new(UnitKind::Pixels, 3000.0, 2000.0, 300.0);
        let document = GuideRect::new(0.0, 0.0, 3000.0, 2000.0);
        let mut values = GuideLayoutValues::default();
        let mut is_open = opened.get();
        let shown = GuideLayoutDialog::new(units, document, Bind::bound(&mut values)).opened(Bind::bound(&mut is_open)).show(ui);
        opened.set(is_open);
        seen.set(shown.preview.is_some());
    });
    fixture.settle();
    assert!(previewing.get(), "Preview starts on and the layout previews");
    let (Some(preview), Some(ok)) = (fixture.by_label("Preview"), fixture.by_label("OK")) else {
        panic!("the Preview check box and OK are shown");
    };
    assert!(grows_with(ok.rect.max.y, preview.rect.min.y) && aligned_left(ok.rect, preview.rect), "Preview sits under the buttons");
    fixture.click(&preview);
    fixture.settle();
    assert!(!previewing.get(), "with Preview off the dialog previews nothing");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).idle(4);
    assert!(!open.get(), "Escape closes the dialog");
    // Past its exit fade the closed dialog shows nothing and has ended its shown life: the next open is a first pass.
    fixture.advance(1.0).idle(4);
    open.set(true);
    fixture.settle();
    assert!(previewing.get(), "the next open starts with Preview on");
}

/// With a custom colour chosen, a swatch of it follows the colour list and a click on it asks the host's picker again,
/// at that colour; a named colour shows no swatch.
#[trial]
fn a_click_on_the_custom_colour_swatch_asks_the_picker_at_that_colour() {
    let values = Rc::new(RefCell::new(GuideLayoutValues::new(Default::default(), GuideColor::Custom(Rgba::rgb(10, 200, 30)))));
    let asked: Rc<RefCell<Vec<ColorRequest>>> = Rc::default();
    let (held, seen) = (Rc::clone(&values), Rc::clone(&asked));
    let mut fixture = Fixture::new(move |ui| {
        let units = GuideUnits::new(UnitKind::Pixels, 3000.0, 2000.0, 300.0);
        let document = GuideRect::new(0.0, 0.0, 3000.0, 2000.0);
        let mut values = held.borrow_mut();
        let mut shown = GuideLayoutDialog::new(units, document, Bind::bound(&mut *values)).opened(Bind::given(true)).show(ui);
        seen.borrow_mut().extend(shown.requests.take_color());
    });
    fixture.settle();
    assert!(asked.borrow().is_empty(), "nothing asks before the click");
    let swatch = fixture.by_label("Custom…").expect("the custom colour's swatch follows the list");
    fixture.click(&swatch);
    fixture.settle();
    let asked_now = asked.borrow();
    let [request] = asked_now.as_slice() else { panic!("one colour request, got {}", asked_now.len()) };
    assert_eq!(request.key, GUIDE_LAYOUT_COLOR_KEY);
    assert!(request.color.is_some(), "the picker opens at the custom colour");
    drop(asked_now);

    values.borrow_mut().color = GuideColor::Cyan;
    fixture.settle();
    assert!(fixture.by_label("Custom…").is_none(), "a named colour shows no swatch");
}

const REMEMBERED_GUIDE: SaveKey = SaveKey::new("craft_ui_trials.new_guide");

/// S-D1: OK stores a new guide's axis, position and colour under its `remember` key, and the next open starts from them
/// whatever values the app passes; Cancel stores nothing.
#[trial]
fn a_new_guide_remembers_its_confirmed_values_for_the_next_open() {
    let open = Rc::new(Cell::new(true));
    let values = Rc::new(RefCell::new(NewGuideValues::new(Axis::X, 0.0, GuideColor::Magenta)));
    let made: Rc<RefCell<Vec<Guide>>> = Rc::default();
    let (opened, held, seen) = (Rc::clone(&open), Rc::clone(&values), Rc::clone(&made));
    let mut fixture = Fixture::new(move |ui| {
        let units = GuideUnits::new(UnitKind::Pixels, 3000.0, 2000.0, 300.0);
        let mut is_open = opened.get();
        let mut current = held.borrow_mut();
        let shown = NewGuideDialog::new(units, Bind::bound(&mut *current)).remember(REMEMBERED_GUIDE).opened(Bind::bound(&mut is_open)).show(ui);
        opened.set(is_open);
        seen.borrow_mut().extend(shown.result);
    });
    // The app opens the dialog again with its own default values. Past its exit fade the closed dialog shows nothing
    // and has ended its shown life, so the reopen is a first pass.
    let reopen = |fixture: &mut Fixture| {
        fixture.advance(1.0).idle(4);
        *values.borrow_mut() = NewGuideValues::default();
        open.set(true);
        fixture.settle();
    };

    fixture.settle();
    assert_eq!(*values.borrow(), NewGuideValues::new(Axis::X, 0.0, GuideColor::Magenta), "with nothing remembered the opening values stand");
    let vertical = fixture.by_label("Vertical").expect("the orientation offers Vertical");
    fixture.click(&vertical).settle();
    let position = fixture.by_label("Position").expect("the Position field");
    fixture.click(&position).settle();
    fixture.key(egui::Key::A, egui::Modifiers::COMMAND);
    let typed = 120.0;
    fixture.type_text(&typed.to_string());
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE).settle();
    assert!(!open.get(), "Enter is OK");
    let confirmed: Vec<_> = made.borrow().iter().map(|guide| (guide.along, guide.position, guide.color)).collect();
    assert_eq!(confirmed, [(Axis::Y, typed, Some(GuideColor::Magenta.rgba()))], "OK makes the guide of the edited values");

    reopen(&mut fixture);
    assert_eq!(*values.borrow(), NewGuideValues::new(Axis::Y, typed, GuideColor::Magenta), "the next open starts from the confirmed values");
    let horizontal = fixture.by_label("Horizontal").expect("the orientation offers Horizontal");
    fixture.click(&horizontal).settle();
    assert_eq!(values.borrow().along, Axis::X, "the click edits the values");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).idle(4);
    assert!(!open.get(), "Escape cancels");
    assert_eq!(made.borrow().len(), 1, "Cancel makes no guide");

    reopen(&mut fixture);
    assert_eq!(*values.borrow(), NewGuideValues::new(Axis::Y, typed, GuideColor::Magenta), "Cancel stored nothing: the OK'd values come back");
}

/// Position text that does not parse keeps OK from making a guide at the last good position: nothing is made and the
/// dialog stays open until the text is a number.
#[trial]
fn position_text_that_does_not_parse_keeps_ok_from_making_a_guide() {
    let open = Rc::new(Cell::new(true));
    let made: Rc<RefCell<Vec<Guide>>> = Rc::default();
    let enabled = Rc::new(Cell::new(true));
    let (opened, seen, ok_seen) = (Rc::clone(&open), Rc::clone(&made), Rc::clone(&enabled));
    let mut values = NewGuideValues::new(Axis::X, 30.0, GuideColor::Magenta);
    let mut fixture = Fixture::new(move |ui| {
        let units = GuideUnits::new(UnitKind::Pixels, 3000.0, 2000.0, 300.0);
        let mut is_open = opened.get();
        let shown = NewGuideDialog::new(units, Bind::bound(&mut values)).opened(Bind::bound(&mut is_open)).show(ui);
        opened.set(is_open);
        ok_seen.set(shown.ok_enabled);
        seen.borrow_mut().extend(shown.result);
    });
    fixture.settle();
    let position = fixture.by_label("Position").expect("the Position field");
    fixture.click(&position).settle();
    fixture.key(egui::Key::A, egui::Modifiers::COMMAND);
    fixture.type_text("far").settle();
    assert!(!enabled.get(), "OK is disabled while the text does not parse");
    let ok = fixture.by_label("OK").expect("the footer has OK");
    fixture.click(&ok).settle();
    assert!(open.get(), "the dialog stays open");
    assert!(made.borrow().is_empty(), "no guide is made at the last good position");

    let position = fixture.by_label("Position").expect("the Position field");
    fixture.click(&position).settle();
    fixture.key(egui::Key::A, egui::Modifiers::COMMAND);
    let typed = 40.0;
    fixture.type_text(&typed.to_string()).settle();
    assert!(enabled.get(), "a number enables OK again");
    let ok = fixture.by_label("OK").expect("the footer has OK");
    fixture.click(&ok).settle();
    assert!(!open.get(), "OK closes the dialog");
    let confirmed: Vec<_> = made.borrow().iter().map(|guide| guide.position).collect();
    assert_eq!(confirmed, [typed], "OK makes the guide at the typed position");
}
