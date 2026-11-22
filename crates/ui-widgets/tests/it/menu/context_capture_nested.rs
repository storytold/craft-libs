//! One cancelled press closes only its actual top Menu, not its parent Menu family.
use craft_behavior::MotionMode;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::menu::Menu;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Event, Id, Key, Modifiers, Rect};
use std::cell::RefCell;
use std::rc::Rc;
#[derive(Default)]
struct State {
    header: Option<Rect>,
    nested_header: Option<Rect>,
    checkbox: Option<Rect>,
    owner: Option<Id>,
    checked: bool,
    commits: usize,
    outer_closes: Vec<CloseReason>,
    inner_closes: Vec<CloseReason>,
}
fn secondary_open(fixture: &mut Fixture, at: egui::Pos2) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
}
#[trial]
fn held_escape_closes_only_the_actual_top_menu_not_its_parent_and_recovers_fresh() {
    let state = Rc::new(RefCell::new(State::default()));
    let held = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut state = held.borrow_mut();
        let live = !ui.is_sizing_pass() && !craft_ui::runtime::measure::active(ui.ctx());
        let header = ui.button("Real outer section header");
        if live {
            state.header = Some(header.rect);
            state.nested_header = None;
            state.checkbox = None;
            state.owner = None;
        }
        let outer = Menu::context("Outer section options", &header).id_salt("outer-real-menu").show(ui, |ui, _| {
            let nested = ui.button("Real inner section header");
            if !ui.is_sizing_pass() && !craft_ui::runtime::measure::active(ui.ctx()) {
                state.nested_header = Some(nested.rect);
            }
            let inner = Menu::context("Inner section options", &nested).id_salt("inner-real-menu").show(ui, |ui, _| {
                let mut value = state.checked;
                let shown = Checkbox::new(Bind::bound(&mut value)).label("Nested genuine checkbox").id_salt("nested-solo").show(ui);
                if !ui.is_sizing_pass() && !craft_ui::runtime::measure::active(ui.ctx()) {
                    state.checked = value;
                    state.commits += usize::from(shown.events.committed().is_some());
                    state.checkbox = Some(shown.response.interact_rect);
                    state.owner = Some(shown.response.id);
                }
            });
            if !ui.is_sizing_pass()
                && !craft_ui::runtime::measure::active(ui.ctx())
                && let Some(why) = inner.output.closed
            {
                state.inner_closes.push(why);
            }
        });
        if live && let Some(why) = outer.output.closed {
            state.outer_closes.push(why);
        }
    });
    fixture.motion(MotionMode::Off).settle();
    let header = state.borrow().header.expect("real outer opener").center();
    secondary_open(&mut fixture, header);
    let inner_header = state.borrow().nested_header.expect("actual outer body").center();
    secondary_open(&mut fixture, inner_header);
    let (owner, at) = {
        let state = state.borrow();
        (state.owner.expect("actual inner Checkbox"), state.checkbox.expect("actual inner body").center())
    };
    fixture.frame(vec![pointer::moved(at), pointer::press(at)]);
    assert_eq!(craft_ui::runtime::pointer::with_pass_pointer(fixture.ctx(), |p| p.press_targets.click), Some(owner));
    assert!(fixture.ctx().input(|input| input.pointer.primary_down()));
    assert!(!state.borrow().checked);
    assert_eq!(state.borrow().commits, 0);
    fixture.frame(vec![key_press(Key::Escape, Modifiers::NONE)]);
    assert!(fixture.ctx().input(|input| input.pointer.primary_down()));
    assert!(!state.borrow().checked);
    assert_eq!(state.borrow().commits, 0);
    fixture.frame(vec![Event::Key { key: Key::Escape, physical_key: Some(Key::Escape), pressed: false, repeat: false, modifiers: Modifiers::NONE }]);
    assert!(state.borrow().checkbox.is_none(), "the actual top Menu closes by the same keyup boundary");
    assert!(state.borrow().nested_header.is_some(), "its distinct parent Menu remains open");
    assert!(state.borrow().outer_closes.is_empty(), "the cancelled inner press does not dismiss an unrelated parent owner");
    assert_eq!(state.borrow().inner_closes, vec![CloseReason::Escape]);
    fixture.frame(vec![pointer::release(at)]);
    assert!(!state.borrow().checked);
    assert_eq!(state.borrow().commits, 0);
    assert!(state.borrow().nested_header.is_some(), "the old release does not dismiss the parent");
    let again = state.borrow().nested_header.expect("parent retains the real inner opener").center();
    secondary_open(&mut fixture, again);
    let fresh = state.borrow().checkbox.expect("fresh real reopen").center();
    fixture.click_at(fresh);
    assert!(state.borrow().checked);
    assert_eq!(state.borrow().commits, 1);
}
