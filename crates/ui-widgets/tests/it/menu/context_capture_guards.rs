//! Physical layer-host policy controls. The unchanged context_capture test remains the canonical public Menu oracle.
use craft_behavior::{CancelRule, MotionMode, Sense};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{
    Bind, Common, Component, Container, ContainerProps, Cx, Mount, NoValue, Outcome, Overlay, Props as _, Role, Show as _, ShowContainer as _,
    Trigger,
};
use craft_ui::runtime::focus::{FocusPolicy, Initial};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::style::StyleCx;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Event, Id, Key, Modifiers, Rect, Response, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

// A test-only public container supplies policy variants the Menu builder intentionally keeps private. The shared
// host, real native opener and genuine Checkbox lifecycle are unmodified; no press or popup state is injected.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct PolicyLayer {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    closed: Option<CloseReason>,
}
struct PolicyOut {
    response: Response,
    closed: Option<CloseReason>,
}
impl Outcome for PolicyOut {
    type Value = NoValue;
    fn response(&self) -> &Response {
        &self.response
    }
    fn unshown(response: Response) -> Option<Self> {
        Some(Self { response, closed: None })
    }
}
impl Component for PolicyLayer {
    const ROLE: Role = Role::GROUP;
    const STOPS: bool = false;
    type Style = PlainStyle;
    type Output = PolicyOut;
    fn style(_: &StyleCx<'_>, _: PolicyLayerStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(210.0, 60.0))
    }
    fn sense(&self) -> Sense {
        Sense::empty()
    }
    fn finish(self, cx: Cx<'_, Self>) -> PolicyOut {
        PolicyOut { response: cx.response.clone(), closed: self.closed }
    }
}
impl Container for PolicyLayer {
    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}
#[derive(Default)]
struct State {
    header: Option<Rect>,
    checkbox: Option<Rect>,
    owner: Option<Id>,
    checked: bool,
    commits: usize,
    closes: Vec<CloseReason>,
}
fn open(fixture: &mut Fixture, state: &Rc<RefCell<State>>) {
    let at = state.borrow().header.expect("genuine native header").center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
    assert!(state.borrow().checkbox.is_some(), "the actual opener displays the Checkbox in the policy layer");
}
fn held_escape_guard(policy: FocusPolicy, modal_rank: bool, closes: bool) {
    let state = Rc::new(RefCell::new(State::default()));
    let held = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut state = held.borrow_mut();
        let live = !ui.is_sizing_pass() && !craft_ui::runtime::measure::active(ui.ctx());
        let header = ui.button("Real policy header");
        if live {
            state.header = Some(header.rect);
            state.checkbox = None;
            state.owner = None;
        }
        let overlay = if modal_rank { Overlay::dialog() } else { Overlay::menu() };
        let props = ContainerProps::new().mount(Mount::Layer(overlay.focus(policy).opened_by(&header, Trigger::Context)));
        let output = PolicyLayer { common: Common::new(), props, closed: None }.id_salt("policy-layer").show(ui, |ui, _| {
            let mut value = state.checked;
            let shown = Checkbox::new(Bind::bound(&mut value)).label("Physical policy checkbox").id_salt("policy-checkbox").show(ui);
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
    open(&mut fixture, &state);
    let (owner, at) = {
        let state = state.borrow();
        (state.owner.expect("actual response"), state.checkbox.expect("actual child").center())
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
    assert_eq!(state.borrow().checkbox.is_none(), closes, "only a Close Menu may close after its own cancelled press");
    assert_eq!(state.borrow().closes, if closes { vec![CloseReason::Escape] } else { vec![] });
    fixture.frame(vec![pointer::release(at)]);
    assert!(!state.borrow().checked, "the cancelled old release does not toggle even when the layer stays open");
    assert_eq!(state.borrow().commits, 0);
    if closes {
        open(&mut fixture, &state);
    }
    let fresh = state.borrow().checkbox.expect("the retained or freshly reopened body").center();
    fixture.click_at(fresh);
    assert!(state.borrow().checked, "fresh physical input still works");
    assert_eq!(state.borrow().commits, 1);
}
#[trial]
fn close_menu_observes_its_own_completed_escape_cancel() {
    held_escape_guard(FocusPolicy::MENU.initial(Initial::Default), false, true);
}
#[trial]
fn ignore_menu_cancels_the_child_but_does_not_close_or_replay() {
    held_escape_guard(FocusPolicy::MENU.initial(Initial::Default).cancel(CancelRule::Ignore), false, false);
}
#[trial]
fn clear_then_close_menu_does_not_promote_capture_escape_to_close() {
    held_escape_guard(FocusPolicy::MENU.initial(Initial::Default).cancel(CancelRule::ClearThenClose), false, false);
}
#[trial]
fn popup_policy_keeps_existing_capture_priority_and_recovers_fresh() {
    held_escape_guard(FocusPolicy::POPUP, false, false);
}
#[trial]
fn dialog_policy_keeps_existing_capture_priority_and_recovers_fresh() {
    held_escape_guard(FocusPolicy::DIALOG, true, false);
}
