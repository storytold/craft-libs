//! The styles panel's click rules: a click applies, Alt-click clears overrides, a double click opens Options, except on
//! the character style None (unless the app makes character style 0 a base style); at a bare caret every attempt to
//! apply a character style warns.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::Selection;
use craft_text::ui::style_panel::{TextStyleAction, TextStyleKind, TextStyleRow, TextStylesPanel};
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers};

/// The selection the panel shows, the actions it reported and the warnings it raised.
type PanelState = Rc<RefCell<(Selection<u64>, Vec<TextStyleAction>, usize)>>;

#[trial]
fn a_click_applies_alt_clears_overrides_and_a_double_click_opens_options_except_on_none() {
    let state = Rc::new(RefCell::new((Selection::<u64>::default(), Vec::<TextStyleAction>::new())));
    let shared = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let rows = [TextStyleRow::new(0, "None"), TextStyleRow::new(1, "Emphasis"), TextStyleRow::new(2, "Strong")];
        let mut guard = shared.borrow_mut();
        let (selection, actions) = &mut *guard;
        let out = TextStylesPanel::new("Character Styles", TextStyleKind::Character, &rows, Bind::bound(selection)).target(true).show(ui);
        actions.extend(out.action);
    });
    fixture.idle(10);
    let taken = |state: &Rc<RefCell<(Selection<u64>, Vec<TextStyleAction>)>>| std::mem::take(&mut state.borrow_mut().1);

    let emphasis = fixture.by_label("Emphasis").expect("the Emphasis row is shown");
    fixture.click(&emphasis).idle(4);
    assert_eq!(taken(&state), vec![TextStyleAction::apply(1, false)], "a click applies the style");

    let strong = fixture.by_label("Strong").expect("the Strong row is shown");
    fixture.hold(Modifiers::ALT);
    fixture.click(&strong).idle(4);
    fixture.hold(Modifiers::NONE);
    assert_eq!(taken(&state), vec![TextStyleAction::apply(2, true)], "an Alt-click applies and clears the overrides");

    fixture.click(&strong).click(&strong).idle(4);
    assert!(taken(&state).contains(&TextStyleAction::Options(2)), "a double click opens Style Options");

    let none = fixture.by_label("None").expect("the None row is shown");
    fixture.click(&none).click(&none).idle(4);
    assert!(!taken(&state).contains(&TextStyleAction::Options(0)), "a double click on None opens nothing");
}

#[trial]
fn alt_on_new_asks_for_a_name() {
    let state = Rc::new(RefCell::new((Selection::<u64>::default(), Vec::<TextStyleAction>::new())));
    let shared = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let rows = [TextStyleRow::new(0, "None"), TextStyleRow::new(1, "Emphasis")];
        let mut guard = shared.borrow_mut();
        let (selection, actions) = &mut *guard;
        let out = TextStylesPanel::new("Character Styles", TextStyleKind::Character, &rows, Bind::bound(selection)).target(true).show(ui);
        actions.extend(out.action);
    });
    fixture.idle(10);
    let new = fixture.by_label("Create new style (from the selected type)").expect("the New footer button");
    fixture.click(&new).idle(4);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![TextStyleAction::new_style(true, false)], "a plain click does not ask");
    fixture.advance(1.0).hold(Modifiers::ALT);
    fixture.click(&new).idle(4);
    fixture.hold(Modifiers::NONE);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![TextStyleAction::new_style(true, true)], "an Alt-click asks for a name");
}

#[trial]
fn every_attempt_to_apply_a_character_style_at_a_bare_caret_warns_and_applies_nothing() {
    let state = Rc::new(RefCell::new((Selection::<u64>::default(), Vec::<TextStyleAction>::new(), 0usize)));
    let shared = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let rows = [TextStyleRow::new(0, "None"), TextStyleRow::new(1, "Emphasis")];
        let mut guard = shared.borrow_mut();
        let (selection, actions, warnings) = &mut *guard;
        let out = TextStylesPanel::new("Character Styles", TextStyleKind::Character, &rows, Bind::bound(selection))
            .target(true)
            .text_selected(false)
            .show(ui);
        actions.extend(out.action);
        *warnings += usize::from(out.status.is_some());
    });
    fixture.idle(10);
    let warnings = |state: &PanelState| std::mem::take(&mut state.borrow_mut().2);

    let emphasis = fixture.by_label("Emphasis").expect("the Emphasis row is shown");
    fixture.click(&emphasis).idle(4);
    assert_eq!(warnings(&state), 1, "a click warns");
    fixture.advance(1.0).click(&emphasis).idle(4);
    assert_eq!(warnings(&state), 1, "a click on the row already selected warns again");
    fixture.key(Key::Enter, Modifiers::NONE).idle(4);
    assert_eq!(warnings(&state), 1, "Enter on the row warns");
    assert!(state.borrow().1.is_empty(), "nothing is applied at a bare caret");
}

/// What the item `item` of the menu a right-click on `row` opens reports.
fn menu_ran(fixture: &mut Fixture, state: &Rc<RefCell<(Selection<u64>, Vec<TextStyleAction>)>>, row: &str, item: &str) -> Vec<TextStyleAction> {
    let at = fixture.by_label(row).expect("the row is shown").center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.idle(2);
    let rows = fixture.by_role(Role::MENU_ITEM);
    let found = rows.iter().find(|target| fixture.tree().name_of(target.node) == Some(item)).copied();
    fixture.click(&found.unwrap_or_else(|| panic!("the menu has {item:?}"))).idle(4);
    std::mem::take(&mut state.borrow_mut().1)
}

/// A right-click on a row opens its menu: Redefine Style redefines that row's style, not the current one the footer's
/// Redefine takes, Clear Override applies it without the overrides, and on the character style None Redefine Style
/// does nothing.
#[trial]
fn a_rows_menu_redefines_and_clears_the_overrides_of_that_rows_style() {
    let state = Rc::new(RefCell::new((Selection::<u64>::default(), Vec::<TextStyleAction>::new())));
    let shared = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let rows = [TextStyleRow::new(0, "None"), TextStyleRow::new(1, "Emphasis"), TextStyleRow::new(2, "Strong")];
        let mut guard = shared.borrow_mut();
        let (selection, actions) = &mut *guard;
        let out = TextStylesPanel::new("Character Styles", TextStyleKind::Character, &rows, Bind::bound(selection))
            .current(Some(1), true)
            .target(true)
            .show(ui);
        actions.extend(out.action);
    });
    fixture.idle(10);
    assert_eq!(menu_ran(&mut fixture, &state, "Strong", "Redefine Style"), vec![TextStyleAction::Redefine(2)], "the row's style is redefined");
    assert_eq!(
        menu_ran(&mut fixture, &state, "Strong", "Clear Override"),
        vec![TextStyleAction::apply(2, true)],
        "the row's style applies without overrides"
    );

    let redefine = fixture.by_label("Redefine style by current selection").expect("the footer's Redefine");
    fixture.click(&redefine).idle(4);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![TextStyleAction::Redefine(1)], "the footer redefines the current style");
    assert_eq!(menu_ran(&mut fixture, &state, "None", "Redefine Style"), vec![], "None is never redefined");
}

/// The selection the panel shows and the actions it reported.
type Actions = Rc<RefCell<(Selection<u64>, Vec<TextStyleAction>)>>;

/// The tip every editable row describes itself with.
const ROW_TIP: &str = "Click to apply, Alt-click to clear overrides, double-click for Style Options";

/// A Character Styles panel over a style 0 named `zero`, with style 1 current and overridden; `base` turns on
/// [`TextStylesPanel::base_character_style`].
fn character_panel(zero: &'static str, base: bool) -> (Fixture, Actions) {
    let state = Rc::new(RefCell::new((Selection::<u64>::default(), Vec::<TextStyleAction>::new())));
    let shared = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let rows = [TextStyleRow::new(0, zero), TextStyleRow::new(1, "Emphasis")];
        let mut guard = shared.borrow_mut();
        let (selection, actions) = &mut *guard;
        let out = TextStylesPanel::new("Character Styles", TextStyleKind::Character, &rows, Bind::bound(selection))
            .current(Some(1), true)
            .target(true)
            .base_character_style(base)
            .show(ui);
        actions.extend(out.action);
    });
    fixture.idle(10);
    (fixture, state)
}

/// The description the row named `name` carries.
fn description(fixture: &Fixture, name: &str) -> Option<String> {
    let row = fixture.by_label(name).expect("the row is shown");
    fixture.tree().description_of(row.node).map(str::to_owned)
}

/// By default character style 0 is "No character style": its row says so, and neither its menu, a double click nor the
/// footer opens its options, duplicates it or deletes it.
#[trial]
fn character_style_none_takes_no_options_duplicate_or_delete() {
    let (mut fixture, state) = character_panel("None", false);
    assert_eq!(description(&fixture, "None").as_deref(), Some("No character style"), "the row describes itself as no style");

    let none = fixture.by_label("None").expect("the None row is shown");
    fixture.click(&none).click(&none).idle(4);
    std::mem::take(&mut state.borrow_mut().1);
    let options = fixture.by_label("Style Options…").expect("the footer's Style Options");
    fixture.click(&options).idle(4);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![], "the footer opens no options for None");
    let duplicate = fixture.by_label("Duplicate style").expect("the footer's Duplicate");
    fixture.click(&duplicate).idle(4);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![], "the footer does not duplicate None");

    assert_eq!(menu_ran(&mut fixture, &state, "None", "Style Options…"), vec![], "the menu opens no options for None");
}

/// With `base_character_style`, character style 0 is a style like the others: a double click, the footer and its menu
/// open its options, duplicate it and redefine it, and its row carries the editable rows' tip; it is still never deleted.
#[trial]
fn a_base_character_style_takes_options_duplicate_and_redefine_but_not_delete() {
    let (mut fixture, state) = character_panel("Normal", true);
    assert_eq!(description(&fixture, "Normal").as_deref(), Some(ROW_TIP), "the row describes itself as editable");

    let normal = fixture.by_label("Normal").expect("the Normal row is shown");
    fixture.click(&normal).click(&normal).idle(4);
    assert!(std::mem::take(&mut state.borrow_mut().1).contains(&TextStyleAction::Options(0)), "a double click opens Style Options");
    let options = fixture.by_label("Style Options…").expect("the footer's Style Options");
    fixture.click(&options).idle(4);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![TextStyleAction::Options(0)], "the footer opens its options");
    let duplicate = fixture.by_label("Duplicate style").expect("the footer's Duplicate");
    fixture.click(&duplicate).idle(4);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![TextStyleAction::Duplicate(0)], "the footer duplicates it");
    let delete = fixture.by_label("Delete style").expect("the footer's Delete");
    fixture.click(&delete).idle(4);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![], "the footer does not delete it");
    fixture.advance(1.0).click(&normal).idle(4);
    std::mem::take(&mut state.borrow_mut().1);
    fixture.key(Key::Delete, Modifiers::NONE).idle(4);
    assert_eq!(std::mem::take(&mut state.borrow_mut().1), vec![], "the Delete key does not delete it");

    assert_eq!(menu_ran(&mut fixture, &state, "Normal", "Style Options…"), vec![TextStyleAction::Options(0)], "the menu opens its options");
    assert_eq!(menu_ran(&mut fixture, &state, "Normal", "Duplicate Style"), vec![TextStyleAction::Duplicate(0)], "the menu duplicates it");
    assert_eq!(menu_ran(&mut fixture, &state, "Normal", "Redefine Style"), vec![TextStyleAction::Redefine(0)], "the menu redefines it");
    assert_eq!(menu_ran(&mut fixture, &state, "Normal", "Delete Style"), vec![], "the menu does not delete it");
}
