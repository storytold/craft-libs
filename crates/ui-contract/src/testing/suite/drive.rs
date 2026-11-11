//! The drivers and the value cases: each way of changing a story's value runs in its own fixture and must end the
//! gesture once, Escape restores, a double-click resets with one `Set` and a mixed value commits a concrete one.

use super::{ANY, Case, Subject};
use crate::contract::Probe;
use crate::runtime::access::Role;
use crate::runtime::egui::access::{Declared, NodeKey};
use crate::runtime::egui::fixture::AccessActionKind;
use crate::testing::fixture::{Fixture, Target};
use crate::testing::story::{Drive, EditKind, Grip, Required, Story};
use craft_behavior::{Input, MotionMode, Sense};
use egui::{Id, Key, Modifiers, Pos2, Vec2};

pub(super) const CASES: &[Case] = &[
    Case::new("values", ANY, a_drive_ends_each_gesture_once),
    Case::new("reset", ANY, a_double_click_resets_with_one_set),
    Case::new("mixed", ANY, mixed_commits_a_concrete_value),
];

/// A drive with the library's choice made.
#[derive(Clone, Copy)]
pub(super) enum Plan {
    Activate,
    Step,
    Drag(Grip),
    DragChild(Grip),
    /// Types `text` into the subject; `story` names the story in the failure when focus never reaches it.
    Type(&'static str, &'static str),
    Pick(usize),
    Custom(fn(&mut Fixture)),
    Never,
}

/// The drive a story names, or the library's choice from the type's facts and the settled fixture's shape record
/// (22.4 drivers).
pub(super) fn plan(story: &Story, probe: &Probe, fixture: &Fixture) -> Plan {
    match story.drive {
        Drive::Auto if probe.carries_value() && !probe.takes_input() => Plan::Never,
        Drive::Auto if matches!(probe.input, Input::Text | Input::Editor) => Plan::Type("sample", story.name),
        Drive::Auto if probe.overlay => Plan::Pick(1),
        Drive::Auto if probe.carries_value() => match fixture.drag_part() {
            Some(part) => Plan::Drag(Grip::part(part)),
            None if probe.keys.activate != crate::runtime::focus::ActivateKeys::Neither => Plan::Activate,
            None => Plan::Step,
        },
        Drive::Auto | Drive::Activate => Plan::Activate,
        Drive::Step => Plan::Step,
        Drive::Drag(grip) => Plan::Drag(grip),
        Drive::DragChild(grip) => Plan::DragChild(grip),
        Drive::Type(text) => Plan::Type(text, story.name),
        Drive::Pick { row } => Plan::Pick(row),
        Drive::Custom(custom) => Plan::Custom(custom),
        _ => Plan::Never,
    }
}

/// The ways `plan` runs, by variant number: the activation keys the type keeps, a step by AccessKit and by key,
/// a drag and a drag cancelled by Escape, a pick by keys and by a click on the row.
pub(super) fn variants(plan: Plan, probe: &Probe) -> Vec<usize> {
    match plan {
        Plan::Activate => {
            let mut runs = vec![0];
            if probe.keys.activate.enter() {
                runs.push(1);
            }
            if probe.keys.activate.space() {
                runs.push(2);
            }
            runs.push(3);
            runs
        }
        Plan::Step | Plan::Drag(_) | Plan::DragChild(_) | Plan::Pick(_) => vec![0, 1],
        Plan::Type(..) | Plan::Custom(_) => vec![0],
        Plan::Never => Vec::new(),
    }
}

/// Whether the variant ends by cancelling, so the value must be as it was.
pub(super) fn cancels(plan: Plan, variant: usize) -> bool {
    matches!(plan, Plan::Drag(_) | Plan::DragChild(_)) && variant == 1
}

pub(super) fn variant_name(plan: Plan, variant: usize) -> &'static str {
    match (plan, variant) {
        (Plan::Activate, 0) => "click",
        (Plan::Activate, 1) => "Enter",
        (Plan::Activate, 2) => "Space",
        (Plan::Activate, _) => "AccessKit Click",
        (Plan::Step, 0) => "AccessKit Increment",
        (Plan::Step, _) => "forward key",
        (Plan::Drag(_) | Plan::DragChild(_), 0) => "drag",
        (Plan::Drag(_) | Plan::DragChild(_), _) => "drag with Escape",
        (Plan::Pick(_), 0) => "pick by keys",
        (Plan::Pick(_), _) => "pick by click",
        (Plan::Type(..), _) => "typed text",
        (Plan::Custom(_), _) => "custom drive",
        (Plan::Never, _) => "no drive",
    }
}

/// The node of the shown component, or the whole shown area when it has none.
pub(super) fn root_target(fixture: &Fixture) -> Target {
    fixture.root_id().and_then(|id| fixture.node_of(id)).unwrap_or(Target { node: NodeKey::of(Id::NULL), rect: fixture.shown() })
}

/// The node a click or an AccessKit `Click` activates: the first clickable part, or the component itself.
fn activation_target(fixture: &Fixture) -> Target {
    let part = fixture.click_part().zip(fixture.root_id()).and_then(|(part, id)| fixture.part(id, part));
    part.unwrap_or_else(|| root_target(fixture))
}

/// Activation by the `variant`th way: click, Enter, Space or AccessKit `Click`.
pub(super) fn activate(fixture: &mut Fixture, variant: usize, target: &Target) {
    let none = Modifiers::NONE;
    match variant {
        0 => {
            fixture.click(target);
        }
        1 | 2 => {
            fixture.key(Key::Tab, none);
            fixture.key(if variant == 1 { Key::Enter } else { Key::Space }, none);
            fixture.settle();
        }
        _ => {
            fixture.access(AccessActionKind::Click, target);
            fixture.settle();
        }
    }
}

fn step(fixture: &mut Fixture, variant: usize, target: &Target) {
    if variant == 0 {
        // A component whose value lives in sub-nodes (thumbs, axes) steps through the first of them.
        let tree = fixture.tree();
        let part = fixture.by_role(Role::SLIDER).into_iter().find(|slider| tree.declares_key(slider.node, Declared::Increment));
        let target = if tree.declares_key(target.node, Declared::Increment) { *target } else { part.unwrap_or(*target) };
        fixture.access(AccessActionKind::Increment, &target);
        fixture.settle();
    } else {
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.key(Key::ArrowRight, Modifiers::NONE);
        fixture.settle();
    }
}

/// A drag from `from` by twice the slop along `toward`, ended by a release or, with `escape`, by Escape before it.
fn drag(fixture: &mut Fixture, from: Pos2, toward: Vec2, escape: bool) {
    let by = 2.0 * crate::testing::input::max_click_dist(fixture.ctx()) * toward;
    if !escape {
        fixture.drag_at(from, by);
        return;
    }
    fixture.frame(vec![crate::testing::pointer::moved(from)]);
    fixture.frame(vec![crate::testing::pointer::press(from)]);
    for at in 1..=4 {
        fixture.frame(vec![crate::testing::pointer::moved(from + by * (at as f32 / 4.0))]);
    }
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.frame(vec![crate::testing::pointer::release(from + by)]);
    fixture.settle();
}

/// The rows a popup offers: the options, menu entries, tree items and list items (a `TileGrid` tile) on screen.
fn pickable(fixture: &Fixture) -> Vec<Target> {
    [Role::LIST_BOX_OPTION, Role::MENU_ITEM, Role::TREE_ITEM, Role::LIST_ITEM].into_iter().flat_map(|role| fixture.by_role(role)).collect()
}

fn pick(fixture: &mut Fixture, row: usize, by_click: bool) {
    let none = Modifiers::NONE;
    let target = root_target(fixture);
    // Rows the component shows before its popup opens are not the popup's.
    let before: Vec<NodeKey> = pickable(fixture).into_iter().map(|row| row.node).collect();
    fixture.click(&target);
    if by_click {
        let mut rows: Vec<Target> = pickable(fixture).into_iter().filter(|row| row.node != target.node && !before.contains(&row.node)).collect();
        rows.sort_by(|a, b| (a.rect.min.y, a.rect.min.x).partial_cmp(&(b.rect.min.y, b.rect.min.x)).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(row) = rows.get(row).copied() {
            super::row_click::click(fixture, &row);
        }
    } else {
        for _ in 0..row {
            fixture.key(Key::ArrowDown, none);
        }
        fixture.key(Key::Enter, none);
    }
    fixture.settle();
}

/// Presses Tab until the focus is inside the subject (its root or a descendant), for at most one cycle of the stops.
/// A story whose first stops belong to something else (an editor inside a canvas) still gets its text. `false` when
/// the cycle ends without the subject: a subject Tab cannot reach still fails.
///
/// The focus is inside when the focus service's stop is the subject's component or AccessKit's focused node lies in
/// the subject's node subtree.
pub(super) fn focus_subject(fixture: &mut Fixture) -> bool {
    let Some(root_id) = fixture.root_id() else {
        fixture.key(Key::Tab, Modifiers::NONE);
        return fixture.focused().is_some();
    };
    let root = super::nodes::root(fixture);
    let mut seen = Vec::new();
    loop {
        let stop = fixture.focused_stop().map(|(id, _)| id);
        let focused = fixture.tree().focused_key();
        if stop == Some(root_id) || root.as_ref().is_some_and(|root| super::nodes::focus_within(fixture, root.node)) {
            return true;
        }
        let at = (stop, focused);
        if seen.contains(&at) {
            return false;
        }
        seen.push(at);
        fixture.key(Key::Tab, Modifiers::NONE);
    }
}

/// Runs the `variant`th way of `plan` on a settled fixture.
pub(super) fn run(fixture: &mut Fixture, plan: Plan, variant: usize) {
    let target = root_target(fixture);
    match plan {
        Plan::Activate => activate(fixture, variant, &activation_target(fixture)),
        Plan::Step => step(fixture, variant, &target),
        Plan::Drag(grip) => {
            let pressed = fixture.root_id().and_then(|id| fixture.part(id, grip.part)).unwrap_or(target);
            drag(fixture, pressed.center(), grip.toward, variant == 1);
        }
        Plan::DragChild(grip) => {
            let owner = fixture.root_id().unwrap_or(Id::NULL);
            let from = crate::runtime::pointer::child_part_bounds(fixture.ctx(), owner, grip.part, Sense::DRAG)
                .map_or(target.center(), |rect| rect.center());
            drag(fixture, from, grip.toward, variant == 1);
        }
        Plan::Type(text, story) => {
            // A disabled subject takes no focus and no text, which the disabled case checks.
            let disabled = super::nodes::root(fixture).is_some_and(|root| fixture.tree().data_of(root.node).is_some_and(|node| node.is_disabled()));
            assert!(focus_subject(fixture) || disabled, "{story}: Tab never moved focus into the component, so the typed text has nowhere to go");
            fixture.type_text(text);
            // Enter inserts a line in multi-line text, which Cmd/Ctrl+Enter commits; the node declares which it is.
            let multiline = fixture.tree().role_of(fixture.tree().focused_key()) == Some(Role::MULTILINE_TEXT_INPUT);
            fixture.key(Key::Enter, if multiline { Modifiers::COMMAND } else { Modifiers::NONE });
            fixture.settle();
        }
        Plan::Pick(row) => pick(fixture, row, variant == 1),
        Plan::Custom(custom) => custom(fixture),
        Plan::Never => {}
    }
}

/// A settled fixture of `story` with its drive resolved.
pub(super) fn settled(story: &Story, probe: &Probe) -> (Fixture, Plan) {
    let mut fixture = Fixture::story(story);
    fixture.settle();
    let plan = plan(story, probe, &fixture);
    (fixture, plan)
}

/// `fixture` settled for a case that sends input. Such cases run without motion: tweens jump to their end, so a
/// settle after an action ends once the action has landed rather than when its feedback has finished playing. The
/// motion case drives stories under each motion mode itself.
pub(super) fn for_input(mut fixture: Fixture) -> Fixture {
    fixture.motion(MotionMode::Off).settle();
    fixture
}

/// A fixture of `story`, settled for a case that sends input ([`for_input`]).
pub(super) fn input_fixture(story: &Story) -> Fixture {
    for_input(Fixture::story(story))
}

/// What the runs of one story's drive did.
struct Runs {
    /// Every run, by name, in order.
    ran: Vec<&'static str>,
    /// Some run changed the value.
    changed: bool,
    failures: Vec<String>,
}

/// Runs each way of `story`'s drive `plan` in its own fixture: a run that changes the value ends its gesture once, a
/// cancelled run leaves the value as it was, and the runs that are not cancelled agree on changing it.
fn runs_of(story: &Story, probe: &Probe, plan: Plan) -> Runs {
    let mut runs = Runs { ran: Vec::new(), changed: false, failures: Vec::new() };
    let mut changed_by = Vec::new();
    let mut unchanged_by = Vec::new();
    for variant in variants(plan, probe) {
        let mut fixture = input_fixture(story);
        let before = fixture.data().snapshot();
        fixture.take_outputs();
        run(&mut fixture, plan, variant);
        let changed = !fixture.data().changed_since(&before).is_empty();
        let kinds: Vec<EditKind> = fixture.take_outputs().into_iter().flat_map(|output| output.edits).collect();
        let name = variant_name(plan, variant);
        runs.ran.push(name);
        if cancels(plan, variant) {
            if changed || kinds.iter().any(|kind| matches!(kind, EditKind::Commit | EditKind::Set)) {
                runs.failures.push(format!("{}: {name} left a changed value or a commit: {kinds:?}", story.name));
            }
            continue;
        }
        if !changed {
            unchanged_by.push(name);
            continue;
        }
        changed_by.push(name);
        let ends = kinds.iter().filter(|kind| matches!(kind, EditKind::Commit | EditKind::Set)).count();
        if ends != 1 || kinds.contains(&EditKind::Cancel) {
            runs.failures.push(format!("{}: {name} gave {kinds:?}", story.name));
        }
    }
    if !changed_by.is_empty() {
        runs.changed = true;
        if !unchanged_by.is_empty() {
            runs.failures.push(format!("{}: {unchanged_by:?} changed nothing while {changed_by:?} did", story.name));
        }
    }
    runs
}

/// The `values` case over one story of the type `probe` describes: each way of the story's drive runs in its own
/// fixture. `Ok` names the runs in order (a drag is followed by `"drag with Escape"`); `Err` gives the failures, or
/// says that no run changed the value.
pub fn values_case(probe: &Probe, story: &Story) -> Result<Vec<&'static str>, String> {
    let (_, plan) = settled(story, probe);
    let runs = runs_of(story, probe, plan);
    if !runs.failures.is_empty() {
        return Err(runs.failures.join("; "));
    }
    if !runs.changed {
        return Err(format!("{}: no drive changed the value", story.name));
    }
    Ok(runs.ran)
}

fn a_drive_ends_each_gesture_once(subject: &Subject) -> Result<(), String> {
    if !subject.probe.carries_value() {
        return Ok(());
    }
    let mut changed_any = false;
    let mut failures = Vec::new();
    for story in &subject.stories {
        let runs = runs_of(story, subject.probe, subject.base(story).plan);
        changed_any |= runs.changed;
        failures.extend(runs.failures);
    }
    if !changed_any {
        failures.push("no drive changed the value".to_owned());
    }
    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

/// A double-click on the thumb of a dragged control that resets its value gives one `Set`, with no `Begin`,
/// `Update` or `Commit` beside it. Controls with no reset value change nothing and pass.
fn a_double_click_resets_with_one_set(subject: &Subject) -> Result<(), String> {
    if !subject.probe.carries_value() {
        return Ok(());
    }
    for story in &subject.stories {
        let plan = subject.base(story).plan;
        let Plan::Drag(Grip { part, .. }) = plan else { continue };
        let mut fixture = input_fixture(story);
        run(&mut fixture, plan, 0);
        let Some(grip) = fixture.root_id().and_then(|id| fixture.part(id, part)) else { continue };
        let before = fixture.data().snapshot();
        fixture.take_outputs();
        fixture.click(&grip);
        if !fixture.data().changed_since(&before).is_empty() {
            continue;
        }
        fixture.click(&grip);
        if fixture.data().changed_since(&before).is_empty() {
            continue;
        }
        let kinds: Vec<EditKind> = fixture.take_outputs().into_iter().flat_map(|output| output.edits).collect();
        if kinds != [EditKind::Set] {
            return Err(format!("{}: a double-click reset gave {kinds:?}", story.name));
        }
    }
    Ok(())
}

/// A story that shows a mixed value reports it as mixed, and after its drive commits a value the shown value is
/// concrete.
fn mixed_commits_a_concrete_value(subject: &Subject) -> Result<(), String> {
    for story in subject.stories.iter().filter(|story| story.covers.contains(&Required::Mixed)) {
        let is_mixed = |fixture: &Fixture| {
            fixture
                .tree()
                .keys()
                .into_iter()
                .any(|key| fixture.tree().data_of(key).is_some_and(|node| node.toggled() == Some(crate::runtime::egui::access::Toggled::Mixed)))
        };
        let plan = subject.base(story).plan;
        let mut fixture = input_fixture(story);
        if !is_mixed(&fixture) {
            return Err(format!("{}: the mixed story reports no mixed node", story.name));
        }
        let Some(variant) = variants(plan, subject.probe).into_iter().find(|variant| !cancels(plan, *variant)) else { continue };
        let before = fixture.data().snapshot();
        run(&mut fixture, plan, variant);
        if !fixture.data().changed_since(&before).is_empty() && is_mixed(&fixture) {
            return Err(format!("{}: the value is still mixed after the {} committed", story.name, variant_name(plan, variant)));
        }
    }
    Ok(())
}
