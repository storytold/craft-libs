//! Canonical SDK menu ownership is not assumed to fix captured-pointer Escape: exercise the actual public route.
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
    owner: Option<Id>,
    checkbox: Option<Rect>,
    checked: bool,
    commits: usize,
    closes: Vec<CloseReason>,
}
fn open_from_real_header(fixture: &mut Fixture, state: &Rc<RefCell<State>>) {
    let at = state.borrow().header.expect("real native header response").center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    assert!(state.borrow().checkbox.is_some(), "canonical SDK context menu has its genuine Checkbox child");
}

#[trial]
fn canonical_context_owner_closes_on_one_held_checkbox_escape_without_replay_and_recovers_fresh() {
    let state = Rc::new(RefCell::new(State::default()));
    let held = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut state = held.borrow_mut();
        let live = !ui.is_sizing_pass() && !craft_ui::runtime::measure::active(ui.ctx());
        let header = ui.button("Real section header");
        if live {
            state.header = Some(header.rect);
            state.checkbox = None;
            state.owner = None;
        }
        // This REPLACES the native popup owner; it is not an inline menu nested inside a native callback.
        let output = Menu::context("Section options", &header).id_salt("real-header-menu").show(ui, |ui, _| {
            let mut value = state.checked;
            let shown = Checkbox::new(Bind::bound(&mut value)).label("Solo Mode").id_salt("solo").show(ui);
            if !ui.is_sizing_pass() && !craft_ui::runtime::measure::active(ui.ctx()) {
                state.checked = value;
                state.commits += usize::from(shown.events.committed().is_some());
                state.checkbox = Some(shown.response.interact_rect);
                state.owner = Some(shown.response.id);
            }
        });
        if live && let Some(why) = output.output.closed {
            state.closes.push(why);
        }
    });
    fixture.motion(MotionMode::Off).settle();
    open_from_real_header(&mut fixture, &state);
    let (owner, at) = {
        let state = state.borrow();
        (state.owner.expect("published actual Checkbox response"), state.checkbox.expect("genuine child bounds").center())
    };
    fixture.frame(vec![pointer::moved(at), pointer::press(at)]);
    let target = craft_ui::runtime::pointer::with_pass_pointer(fixture.ctx(), |pointer| pointer.press_targets.click);
    assert_eq!(target, Some(owner), "the actual press winner is the genuine non-text Checkbox child");
    assert!(fixture.ctx().input(|input| input.pointer.primary_down()), "physical primary remains held");
    assert!(!state.borrow().checked);
    assert_eq!(state.borrow().commits, 0);
    // Exactly one physical Escape down and its keyup, while primary stays DOWN. No extra Escape or idle frame.
    fixture.frame(vec![key_press(Key::Escape, Modifiers::NONE)]);
    assert!(fixture.ctx().input(|input| input.pointer.primary_down()));
    assert!(!state.borrow().checked);
    assert_eq!(state.borrow().commits, 0);
    fixture.frame(vec![Event::Key { key: Key::Escape, physical_key: Some(Key::Escape), pressed: false, repeat: false, modifiers: Modifiers::NONE }]);
    assert!(state.borrow().checkbox.is_none(), "the genuine SDK owner must close by the original keyup boundary");
    assert_eq!(state.borrow().closes, vec![CloseReason::Escape]);
    fixture.frame(vec![pointer::release(at)]);
    assert!(!state.borrow().checked, "the old held release cannot replay a toggle");
    assert_eq!(state.borrow().commits, 0);
    open_from_real_header(&mut fixture, &state);
    let fresh = state.borrow().checkbox.expect("fresh physical secondary click reopens the real owner").center();
    fixture.click_at(fresh);
    assert!(state.borrow().checked, "fresh real input still works");
    assert_eq!(state.borrow().commits, 1, "one fresh release gives exactly one committed value");
}
