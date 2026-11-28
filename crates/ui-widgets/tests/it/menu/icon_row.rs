//! `MenuIconRow`: a row of icons inside a vertical menu is one stop for Up and Down, Left and Right move between its
//! icons, a disabled icon takes neither focus nor a press, and a pick closes the menu.

use craft_behavior::{IconId, PartRef};
use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _, Trigger};
use craft_ui::menu::{Menu, MenuIcon, MenuIconRow, MenuItem};
use craft_ui::runtime::focus::{FocusTarget, request};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Id, Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The ids the menu's rows had in the latest pass that showed them.
#[derive(Clone, Copy, Default)]
struct Rows {
    target: Option<Id>,
    group: Option<Id>,
    row: Option<Id>,
    ungroup: Option<Id>,
}

/// Regression: a row of icons in a menu took a stop per icon for Up and Down, let Left and Right leave the menu, let a
/// disabled icon take focus or a press, or left the menu open after a pick; a frame run twice reported its pick twice.
#[trial]
fn an_icon_row_is_one_stop_in_the_menu_left_and_right_move_between_its_icons_and_a_pick_closes_the_chain() {
    let rows = Rc::new(Cell::new(Rows::default()));
    let picks = Rc::new(RefCell::new(Vec::new()));
    let twice = Rc::new(Cell::new(false));
    let (seen, picked, discard) = (rows.clone(), picks.clone(), twice.clone());
    let mut fixture = Fixture::new(move |ui| {
        let icons = [
            MenuIcon::new(IconId::new("align_edge_left"), "Align Left Edges"),
            MenuIcon::new(IconId::new("align_edge_hcenter"), "Align Centres").enabled(false),
            MenuIcon::new(IconId::new("align_edge_right"), "Align Right Edges"),
        ];
        let target = Button::new("Canvas").show(ui);
        let mut now = Rows { target: Some(target.id), ..seen.get() };
        Menu::context("Arrange", &target).show(ui, |ui, _| {
            now.group = Some(MenuItem::new("Group").show(ui).response.id);
            let row = MenuIconRow::new("Align", &icons).show(ui);
            now.row = Some(row.response.id);
            picked.borrow_mut().extend(row.picked);
            now.ungroup = Some(MenuItem::new("Ungroup").show(ui).response.id);
        });
        seen.set(now);
        if discard.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial runs the frame twice");
        }
    });
    fixture.settle();
    let open = |fixture: &mut Fixture| {
        // A click does not focus a button, and Shift+F10 asks the focused component for its menu.
        request(fixture.ctx(), FocusTarget::Id(rows.get().target.expect("the menu's target is shown")));
        fixture.settle();
        fixture.key(Key::F10, Modifiers::SHIFT);
        fixture.settle();
        assert!(fixture.by_label("Ungroup").is_some(), "Shift+F10 opens the context menu");
        // Every walk below starts on the first row.
        request(fixture.ctx(), FocusTarget::Id(rows.get().group.expect("the first row is shown")));
        fixture.settle();
    };
    let press = |fixture: &mut Fixture, key: Key| {
        fixture.key(key, Modifiers::NONE);
        fixture.settle();
        fixture.focused_stop()
    };
    open(&mut fixture);
    let Rows { target: target_id, group, row, ungroup } = rows.get();
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), group, "the walk starts on the first row");

    // Down enters the row at its first icon; Right reaches the last enabled icon, past the disabled one, and stops.
    let first = press(&mut fixture, Key::ArrowDown);
    assert_eq!(first.map(|(id, _)| id), row, "Down from the first row lands on the icon row");
    let first_part: Option<PartRef> = first.and_then(|(_, part)| part);
    assert!(first_part.is_some(), "focus is on one of the row's icons");
    let last = press(&mut fixture, Key::ArrowRight);
    assert_eq!(last.map(|(id, _)| id), row, "Right stays in the row");
    assert_ne!(last, first, "Right moves to another icon");
    assert_eq!(press(&mut fixture, Key::ArrowRight), last, "the row has two stops: the disabled icon is none, and Right stops at the end");
    assert!(fixture.by_label("Ungroup").is_some(), "Right at the row's end leaves the menu open");
    assert_eq!(press(&mut fixture, Key::ArrowLeft), first, "Left moves back to the first icon");
    assert_eq!(press(&mut fixture, Key::ArrowLeft), first, "Left stops at the first icon");

    // Up and Down step over the row as one stop, entering it at its first icon.
    assert_eq!(press(&mut fixture, Key::ArrowDown).map(|(id, _)| id), ungroup, "Down from any icon leaves the row for the next row");
    assert_eq!(press(&mut fixture, Key::ArrowUp), first, "Up from the next row enters the row at its first icon");
    assert_eq!(press(&mut fixture, Key::ArrowUp).map(|(id, _)| id), group, "Up from the row leaves it for the row before");

    // Enter on the last icon picks it once, in a frame egui runs twice, and closes the menu.
    press(&mut fixture, Key::ArrowDown);
    press(&mut fixture, Key::ArrowRight);
    twice.set(true);
    fixture.key(Key::Enter, Modifiers::NONE);
    twice.set(false);
    fixture.settle();
    assert_eq!(*picks.borrow(), vec![2], "Enter picks the focused icon once, by its index among all the icons");
    assert!(fixture.by_label("Ungroup").is_none(), "the pick closes the menu");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), target_id, "focus returns to where it was before the menu opened");

    // A click on the disabled icon picks nothing and keeps the menu open; a click on an enabled one picks and closes.
    open(&mut fixture);
    let disabled = fixture.by_label("Align Centres").expect("the disabled icon shows");
    fixture.click(&disabled);
    fixture.settle();
    assert_eq!(*picks.borrow(), vec![2], "the disabled icon takes no press");
    assert!(fixture.by_label("Ungroup").is_some(), "the menu stays open");
    let left = fixture.by_label("Align Left Edges").expect("the first icon shows");
    fixture.click(&left);
    fixture.settle();
    assert_eq!(*picks.borrow(), vec![2, 0], "a click picks the icon under it");
    assert!(fixture.by_label("Ungroup").is_none(), "the pick closes the menu");
}

/// Regression: Return as the first key in a menu the pointer opened picked the first icon of a focused icon row the
/// pointer never lit. With no row lit it only closes the menu; with a row lit under the pointer it runs that row.
#[trial]
fn return_in_a_pointer_opened_menu_whose_first_row_is_an_icon_row_runs_only_the_lit_row() {
    let picks = Rc::new(RefCell::new(Vec::new()));
    let ran = Rc::new(Cell::new(None::<&'static str>));
    let row_id = Rc::new(Cell::new(None::<Id>));
    let (picked, seen, shown) = (picks.clone(), ran.clone(), row_id.clone());
    let mut fixture = Fixture::new(move |ui| {
        let icons =
            [MenuIcon::new(IconId::new("align_edge_left"), "Align Left Edges"), MenuIcon::new(IconId::new("align_edge_right"), "Align Right Edges")];
        let opener = Button::new("Open").show(ui);
        Menu::new("Arrange").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            let row = MenuIconRow::new("Align", &icons).show(ui);
            shown.set(Some(row.response.id));
            picked.borrow_mut().extend(row.picked);
            for name in ["Group", "Ungroup"] {
                if MenuItem::new(name).show(ui).ran {
                    seen.set(Some(name));
                }
            }
        });
    });
    fixture.settle();
    let open = |fixture: &mut Fixture| {
        let opener = fixture.by_label("Open").expect("the opener");
        fixture.advance(1.0).click(&opener).settle();
        assert!(fixture.by_label("Ungroup").is_some(), "a click on the opener opens the menu");
        assert_eq!(fixture.focused_stop().map(|(id, _)| id), row_id.get(), "the menu puts its first row, the icon row, in focus");
    };
    open(&mut fixture);
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert!(fixture.by_label("Ungroup").is_none(), "Return with no row lit closes the menu");
    assert!(picks.borrow().is_empty(), "and picks no icon");
    assert_eq!(ran.get(), None, "and runs no row");
    open(&mut fixture);
    let ungroup = fixture.by_label("Ungroup").expect("the Ungroup row").center();
    fixture.frame(vec![pointer::moved(ungroup)]).settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(ran.get(), Some("Ungroup"), "Return runs the row the pointer lit");
    assert!(picks.borrow().is_empty(), "and picks no icon of the focused row");
    assert!(fixture.by_label("Ungroup").is_none(), "and the run closes the menu");
}

/// Regression: the first key in a menu the pointer opened was withheld from an icon an AccessKit `Focus` had moved to,
/// so Return picked nothing. Only the focus the menu gave as it opened is withheld from; an icon aimed at is picked.
#[trial]
fn return_on_an_icon_accesskit_focused_in_a_pointer_opened_menu_picks_it() {
    let picks = Rc::new(RefCell::new(Vec::new()));
    let picked = picks.clone();
    let mut fixture = Fixture::new(move |ui| {
        let icons =
            [MenuIcon::new(IconId::new("align_edge_left"), "Align Left Edges"), MenuIcon::new(IconId::new("align_edge_right"), "Align Right Edges")];
        let opener = Button::new("Open").show(ui);
        Menu::new("Arrange").opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
            picked.borrow_mut().extend(MenuIconRow::new("Align", &icons).show(ui).picked);
            MenuItem::new("Group").show(ui);
        });
    });
    fixture.settle();
    let opener = fixture.by_label("Open").expect("the opener");
    fixture.advance(1.0).click(&opener).settle();
    let right = fixture.by_label("Align Right Edges").expect("the menu shows its icons");
    fixture.access(AccessActionKind::Focus, &right).settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*picks.borrow(), vec![1], "Return picks the icon AccessKit focused");
    assert!(fixture.by_label("Group").is_none(), "and the pick closes the menu");
}
