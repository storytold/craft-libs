//! `ActionsPanel`: where a dragged row lands.

use std::cell::RefCell;
use std::rc::Rc;

use craft_automation::actions::ActionList;
use craft_automation::ui::actions::mock::{FakeActionsHost, action, list_of};
use craft_automation::ui::actions::{ActionMove, ActionPlace, ActionSelection, ActionsEvent, ActionsPanel, ActionsPanelState, INCLUDE_COLUMN};
use craft_ui::testing::{Fixture, Target, pointer, trial};
use egui::{Key, Modifiers, Pos2, Ui, Vec2};

/// The app around the panel: what it holds and what the panel reported.
struct App {
    state: ActionsPanelState,
    list: ActionList,
    host: FakeActionsHost,
    events: Vec<ActionsEvent>,
}

type Shared = Rc<RefCell<App>>;

/// The room of the panel: wide enough for a row's parts and inside the 800 by 600 screen of a fixture.
const PANEL: Vec2 = Vec2::new(520.0, 520.0);

/// A pointer move that starts a drag.
const DRAG_STEP: Vec2 = Vec2::new(0.0, 8.0);

fn app(list: ActionList) -> App {
    App { state: ActionsPanelState::default(), list, host: FakeActionsHost::prefixed("Run "), events: Vec::new() }
}

fn show(ui: &mut Ui, app: &mut App) {
    let App { state, list, host, events } = app;
    let out = ActionsPanel::new(state, list).host(&*host).show(ui);
    events.extend(out.events);
}

fn rig(app: App) -> (Fixture, Shared) {
    let shared = Rc::new(RefCell::new(app));
    let shown = Rc::clone(&shared);
    let mut fixture = Fixture::new(move |ui| show(ui, &mut shown.borrow_mut()));
    fixture.room(PANEL);
    fixture.settle();
    (fixture, shared)
}

fn take(shared: &Shared) -> Vec<ActionsEvent> {
    std::mem::take(&mut shared.borrow_mut().events)
}

/// The node named `label`.
fn node(fixture: &Fixture, label: &str) -> Target {
    fixture.by_label(label).unwrap_or_else(|| {
        let names: Vec<_> = fixture.tree().keys().into_iter().filter_map(|key| fixture.tree().name_of(key)).collect();
        panic!("no node named {label:?}; the tree has {names:?}")
    })
}

/// The point `share` of the way down `target`, in the middle of its width.
fn at(target: &Target, share: f32) -> Pos2 {
    Pos2::new(target.rect.center().x, target.rect.top() + target.rect.height() * share)
}

/// Press at `from`, move past the drag threshold and on to `to`, release there.
fn drag_to(fixture: &mut Fixture, from: Pos2, to: Pos2) {
    fixture.idle(6);
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    fixture.frame(vec![pointer::moved(from + DRAG_STEP)]);
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![pointer::moved(to)]);
    fixture.frame(vec![pointer::release(to)]);
    fixture.idle(2);
}

/// The moves the panel reported: the dragged row and where it goes.
fn moves(events: &[ActionsEvent]) -> Vec<(ActionSelection, ActionPlace)> {
    events
        .iter()
        .filter_map(|event| match event {
            ActionsEvent::Reorder(ActionMove { row, place, .. }) => Some((row.clone(), place.clone())),
            _ => None,
        })
        .collect()
}

/// Loose "Loose", then the sets "Alpha" (one action) and "Beta" (one action).
fn two_sets() -> ActionList {
    list_of(vec![action("Loose", None, &["a"]), action("First", Some("Alpha"), &["b"]), action("Second", Some("Beta"), &["c"])])
}

/// A drop before a later set goes to the end of the set before it; before the first set it is loose.
#[trial]
fn actions_panel_drop_before_a_later_set_lands_at_the_end_of_the_previous_set() {
    let (mut fixture, shared) = rig(app(two_sets()));
    let (from, to) = (node(&fixture, "Loose").center(), at(&node(&fixture, "Beta"), 0.1));
    drag_to(&mut fixture, from, to);
    assert_eq!(
        moves(&take(&shared)),
        [(ActionSelection::Action(0), ActionPlace::action(1, Some("Alpha".into())))],
        "the drop line sits under Alpha's last action"
    );

    let (from, to) = (node(&fixture, "Second").center(), at(&node(&fixture, "Alpha"), 0.1));
    drag_to(&mut fixture, from, to);
    assert_eq!(
        moves(&take(&shared)),
        [(ActionSelection::Action(2), ActionPlace::action(1, None))],
        "before the first set the action is loose, after the last loose one"
    );
}

/// Space on the focused row flips its Include check.
#[trial]
fn actions_panel_space_flips_the_include_check_of_the_focused_row() {
    let (mut fixture, shared) = rig(app(two_sets()));
    let row = node(&fixture, "Loose").center();
    fixture.frame(vec![pointer::moved(row)]);
    fixture.frame(vec![pointer::press(row)]);
    fixture.frame(vec![pointer::release(row)]);
    fixture.idle(2);
    take(&shared);
    fixture.key(Key::Space, Modifiers::NONE).idle(2);
    let flipped = take(&shared).into_iter().any(|event| {
        matches!(event, ActionsEvent::Toggle(craft_ui::contract::SweepStep::Flip(edit))
            if edit.key == ActionSelection::Action(0) && edit.column == INCLUDE_COLUMN)
    });
    assert!(flipped, "Space flips the row's include cell");
}

/// A step row is a small control high, shorter than its action's row; a set row is a plain row as high as an action's.
#[trial]
fn actions_panel_step_rows_are_shorter_than_action_rows() {
    let mut shown = app(two_sets());
    shown.state.expanded.insert("Loose".into());
    let (fixture, _) = rig(shown);
    let (action, step, set) = (node(&fixture, "Loose").rect, node(&fixture, "Run a").rect, node(&fixture, "Alpha").rect);
    assert!(step.height() < action.height(), "the step row ({step:?}) is shorter than the action row ({action:?})");
    assert_eq!(set.height(), action.height(), "a set row is as high as an action row");
}

/// A click at `pos`, with the pointer moved there first.
fn click(fixture: &mut Fixture, pos: Pos2) {
    fixture.frame(vec![pointer::moved(pos)]);
    fixture.frame(vec![pointer::press(pos)]);
    fixture.frame(vec![pointer::release(pos)]);
    fixture.idle(2);
}

/// In Button Mode every action is a button as wide as the others and the body; a click plays its action once.
#[trial]
fn actions_panel_button_mode_buttons_span_the_body_and_a_click_plays_once() {
    let mut shown = app(two_sets());
    shown.state.settings.button_mode = true;
    let (mut fixture, shared) = rig(shown);
    let buttons: Vec<Target> = ["Loose", "First", "Second"].iter().map(|name| node(&fixture, name)).collect();
    let width = buttons[0].rect.width();
    assert!(buttons.iter().all(|button| button.rect.width() == width), "the buttons share one width: {buttons:?}");
    assert!(width > PANEL.x / 2.0, "a button spans the body, not its name ({width})");
    click(&mut fixture, buttons[1].center());
    let plays: Vec<_> = take(&shared).into_iter().filter(|event| matches!(event, ActionsEvent::Play { .. })).collect();
    assert!(matches!(plays.as_slice(), [ActionsEvent::Play { action: 1, from: 0 }]), "one Play of First: {plays:?}");
}

/// A disabled panel reports nothing for a click on a row or a footer button.
#[trial]
fn actions_panel_disabled_reports_nothing() {
    let shared = Rc::new(RefCell::new(app(two_sets())));
    let shown = Rc::clone(&shared);
    let mut fixture = Fixture::new(move |ui| {
        let App { state, list, host, events } = &mut *shown.borrow_mut();
        events.extend(ActionsPanel::new(state, list).host(&*host).enabled(false).show(ui).events);
    });
    fixture.room(PANEL);
    fixture.settle();
    for name in ["Loose", "Create new set"] {
        let at = node(&fixture, name).center();
        click(&mut fixture, at);
    }
    assert_eq!(take(&shared), [], "a disabled panel reports no event");
    assert_eq!(shared.borrow().state.selected, None, "a click selects no row");
}
