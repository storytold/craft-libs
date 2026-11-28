//! `ActionsPanel`: where a dragged row lands.

use std::cell::RefCell;
use std::rc::Rc;

use craft_automation::actions::ActionList;
use craft_automation::ui::actions::mock::{FakeActionsHost, action, list_of};
use craft_automation::ui::actions::{
    ActionMove, ActionPlace, ActionSelection, ActionsEvent, ActionsPanel, ActionsPanelState, INCLUDE_COLUMN, StepCells,
};
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

fn show(ui: &mut Ui, app: &mut App, cells: StepCells) {
    let App { state, list, host, events } = app;
    let out = ActionsPanel::new(state, list).host(&*host).step_cells(cells).show(ui);
    events.extend(out.events);
}

fn rig(app: App) -> (Fixture, Shared) {
    rig_cells(app, StepCells::default())
}

fn rig_cells(app: App, cells: StepCells) -> (Fixture, Shared) {
    let shared = Rc::new(RefCell::new(app));
    let shown = Rc::clone(&shared);
    let mut fixture = Fixture::new(move |ui| show(ui, &mut shown.borrow_mut(), cells));
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

/// The sets a panel reported duplicating.
fn duplicates(events: &[ActionsEvent]) -> Vec<ActionSelection> {
    events
        .iter()
        .filter_map(|event| match event {
            ActionsEvent::Duplicate(row) => Some(row.clone()),
            _ => None,
        })
        .collect()
}

/// Drags the row labelled `label` onto the footer's Create new action.
fn drag_onto_new_action(fixture: &mut Fixture, label: &str) {
    let (from, to) = (node(fixture, label).center(), node(fixture, "Create new action").center());
    drag_to(fixture, from, to);
}

/// An action row and a step row of the panel's own list dropped on Create new action each report one
/// Duplicate of that row; nothing is taken while recording or playing.
#[trial]
fn a_row_dropped_on_new_action_reports_duplicate() {
    let mut shown = app(two_sets());
    shown.state.expanded.insert("Loose".into());
    let (mut fixture, shared) = rig(shown);
    drag_onto_new_action(&mut fixture, "Loose");
    assert_eq!(duplicates(&take(&shared)), [ActionSelection::Action(0)]);
    drag_onto_new_action(&mut fixture, "Run a");
    assert_eq!(duplicates(&take(&shared)), [ActionSelection::Step(0, 0)]);

    let mut playing = app(two_sets());
    playing.host.playing = true;
    let (mut fixture, shared) = rig(playing);
    drag_onto_new_action(&mut fixture, "Loose");
    assert_eq!(duplicates(&take(&shared)), [], "no drop is taken while playing");

    let mut recording = app(two_sets());
    recording.host.recording = Some(1);
    let (mut fixture, shared) = rig(recording);
    drag_onto_new_action(&mut fixture, "Loose");
    assert_eq!(duplicates(&take(&shared)), [], "no drop is taken while recording");
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

/// The panel menu's last group is Clear All Actions, Reset Actions…, Load Actions…, Replace Actions…, Save Actions…;
/// the three whole-list commands are off while recording, Clear All also with nothing to clear, and each reports
/// its event.
#[trial]
fn actions_panel_menu_offers_the_library_commands() {
    use craft_automation::ui::actions::{ActionsMenuAction, ActionsMenuFacts, ActionsPanelCommands as C, panel_rows};
    use craft_ui::menu::PanelRow;
    use craft_ui::runtime::commands::CommandRef;
    let rows = panel_rows(ActionsMenuFacts::new(), &|_| true);
    let last: Vec<String> = rows.iter().rev().take_while(|row| !matches!(row, PanelRow::Separator)).map(|row| format!("{row:?}")).collect();
    let expected: Vec<String> =
        [C::Save, C::Replace, C::Load, C::Reset, C::ClearAll].into_iter().map(|c| format!("{:?}", PanelRow::Command(CommandRef::from(c)))).collect();
    assert_eq!(last, expected);

    let mut panel = ActionsPanelState::default();
    let list = list_of(vec![action("A", None, &["image.invert"])]);
    let idle = ActionsMenuFacts::new();
    let recording = ActionsMenuFacts::new().recording(true);
    for command in [C::ClearAll, C::Reset, C::Replace] {
        assert!(command.state(&panel, &list, idle).is_enabled() && !command.state(&panel, &list, recording).is_enabled());
    }
    assert!(!C::ClearAll.state(&panel, &ActionList::new(), idle).is_enabled());
    let events: Vec<_> = [C::ClearAll, C::Reset, C::Replace].into_iter().map(|c| c.run(&mut panel, &list, idle)).collect();
    assert_eq!(events, [ActionsEvent::ClearAll, ActionsEvent::Reset, ActionsEvent::Replace].map(|event| Some(ActionsMenuAction::Event(event))));
}

/// A step row has an include and a dialog cell, or with `StepCells::Include` the include cell only; a set row with steps
/// has an include cell whose press reports a toggle of the set, and a set without steps has none.
#[trial]
fn a_set_row_has_an_include_cell_and_the_include_variant_has_no_dialog_column() {
    let mut shown = app(two_sets());
    shown.state.expanded.insert("First".into());
    let (fixture, _) = rig(shown);
    assert!(fixture.by_label("Include in playback: Run b").is_some(), "a step row has its include cell");
    assert!(fixture.by_label("Show dialog during playback: Run b").is_some(), "and its dialog cell");

    let mut list = two_sets();
    list.new_set("Hollow").unwrap();
    let mut shown = app(list);
    shown.state.expanded.insert("First".into());
    let (mut fixture, shared) = rig_cells(shown, StepCells::Include);
    assert!(fixture.by_label("Include in playback: Run b").is_some(), "the include cell stays");
    assert!(fixture.by_label("Show dialog during playback: Run b").is_none(), "the dialog cell is left out");
    assert!(fixture.by_label("Include in playback: Hollow").is_none(), "a set with no steps has no include cell");

    let cell = node(&fixture, "Include in playback: Alpha");
    click(&mut fixture, cell.center());
    let toggled = take(&shared).into_iter().any(|event| {
        matches!(event, ActionsEvent::Toggle(craft_ui::contract::SweepStep::Flip(edit))
            if edit.key == ActionSelection::Set("Alpha".into()) && edit.column == INCLUDE_COLUMN)
    });
    assert!(toggled, "a press on a set's include cell reports a toggle of the set");
}

/// Shift on a later set row selects the sets between, across an expanded set's action rows; Delete reports the range in
/// one event; a click on an action clears the range.
#[trial]
fn shift_clicking_a_set_selects_the_range_and_delete_reports_it_once() {
    let list = list_of(vec![
        action("Loose", None, &["a"]),
        action("First", Some("Alpha"), &["b"]),
        action("Second", Some("Beta"), &["c"]),
        action("Third", Some("Gamma"), &["d"]),
    ]);
    let (mut fixture, shared) = rig(app(list));
    let (first, last) = (node(&fixture, "Alpha").center(), node(&fixture, "Gamma").center());
    click(&mut fixture, first);
    fixture.hold(Modifiers::SHIFT);
    click(&mut fixture, last);
    fixture.hold(Modifiers::NONE);
    let names = ["Alpha", "Beta", "Gamma"].map(str::to_owned).to_vec();
    assert_eq!(shared.borrow().state.selected_sets, names, "the sets between, in list order, not the actions among them");

    take(&shared);
    fixture.key(Key::Delete, Modifiers::NONE).idle(2);
    let events = take(&shared);
    let deletes: Vec<_> = events.iter().filter(|event| matches!(event, ActionsEvent::Delete(_) | ActionsEvent::DeleteSets(_))).collect();
    assert_eq!(deletes, [&ActionsEvent::DeleteSets(names)], "one event for the range");

    let row = node(&fixture, "First").center();
    click(&mut fixture, row);
    assert!(shared.borrow().state.selected_sets.is_empty(), "a click on an action ends the range");
    assert_eq!(shared.borrow().state.selected, Some(ActionSelection::Action(1)));
}

/// A right-click at `pos`, with the pointer moved there first.
fn right_click(fixture: &mut Fixture, pos: Pos2) {
    fixture.frame(vec![pointer::moved(pos)]);
    fixture.frame(vec![pointer::secondary(pos, true)]);
    fixture.frame(vec![pointer::secondary(pos, false)]);
    fixture.idle(2);
}

/// What the row `item` of the menu a right-click on `row` opens reports.
fn menu_ran(fixture: &mut Fixture, shared: &Shared, row: &str, item: &str) -> Vec<ActionsEvent> {
    let at = node(fixture, row).center();
    right_click(fixture, at);
    let at = node(fixture, item).center();
    click(fixture, at);
    take(shared)
}

/// A right-click on an action or a set selects it and opens its menu, whose rows report their event for that row;
/// Play reports nothing while an action plays.
#[trial]
fn a_rows_menu_selects_the_row_and_reports_its_rows_for_it() {
    let (mut fixture, shared) = rig(app(two_sets()));
    assert_eq!(menu_ran(&mut fixture, &shared, "First", "Action Options…"), [ActionsEvent::ActionOptions(1)]);
    assert_eq!(shared.borrow().state.selected, Some(ActionSelection::Action(1)), "the right-click selects the action");
    assert_eq!(menu_ran(&mut fixture, &shared, "Second", "Duplicate"), [ActionsEvent::Duplicate(ActionSelection::Action(2))]);
    assert_eq!(menu_ran(&mut fixture, &shared, "Alpha", "Delete Set"), [ActionsEvent::Delete(ActionSelection::Set("Alpha".into()))]);
    assert_eq!(shared.borrow().state.selected, Some(ActionSelection::Set("Alpha".into())), "the right-click selects the set");

    let mut playing = app(two_sets());
    playing.host.playing = true;
    let (mut fixture, shared) = rig(playing);
    assert_eq!(menu_ran(&mut fixture, &shared, "First", "Play"), [], "Play is off while an action plays");
}
