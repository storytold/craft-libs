//! The styles panel's click rules: a click applies, Alt-click clears overrides, a double click opens Options, except on
//! the character style None; at a bare caret every attempt to apply a character style warns.

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
