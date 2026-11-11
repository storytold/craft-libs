//! The cases every story is shown to: the required states have stories, the story shows and holds still, its
//! AccessKit tree follows the role rules and a focusable story is reachable by Tab.

use super::{ANY, Case, Subject};
use crate::contract::{Probe, ShapeKind};
use crate::testing::access::{Declared, is_interactive, violations};
use crate::testing::bypass;
use crate::testing::fixture::Fixture;
use crate::testing::story::Required;
use craft_behavior::Input;
use egui::{Key, Modifiers};

pub(super) const CASES: &[Case] = &[
    Case::new("required-states", ANY, every_required_state_has_a_story),
    Case::new("shows", ANY, stories_show_and_hold_still),
    Case::new("access", ANY, stories_pass_the_access_rules),
    Case::new("tab", ANY, focusable_stories_are_tab_reachable),
];

/// The exemption that says a type is shown only inside another type's story.
const FACE_ONLY: &str = "face-only";

/// The prefix of an exemption from one required state, as in `state:Mixed`.
const STATE: &str = "state:";

const REQUIRED: [Required; 8] =
    [Required::Default, Required::Mixed, Required::Invalid, Required::LongText, Required::Open, Required::Empty, Required::One, Required::Many];

/// Whether `case` names something a type can be exempt from: a case of the runner, `face-only`, or one state.
pub(super) fn exemption_is_known(case: &str) -> bool {
    super::cases().any(|known| known.name == case)
        || case == FACE_ONLY
        || case.strip_prefix(STATE).is_some_and(|state| REQUIRED.iter().any(|required| format!("{required:?}") == state))
}

/// The states a registered type must show, from the facts of its probe, less the states it is exempt from.
///
/// The invalid state applies to a value a host can refuse: a value the type carries and takes input for, shown with
/// the refused look. A canvas control (`#[component(canvas_control)]`) edits geometry on the canvas and has no
/// refused look, so it has no invalid state.
pub fn required_states(probe: &Probe) -> Vec<Required> {
    let mut states = vec![Required::Default];
    if probe.carries_value() {
        if !probe.single_value {
            states.push(Required::Mixed);
        }
        if probe.takes_input() && !probe.canvas_control {
            states.push(Required::Invalid);
        }
    }
    if matches!(probe.input, Input::Text | Input::Editor) {
        states.push(Required::LongText);
    }
    if probe.overlay {
        states.push(Required::Open);
    }
    if matches!(probe.shape, ShapeKind::Items) {
        states.extend([Required::Empty, Required::One, Required::Many]);
    }
    states.retain(|state| !probe.exempt.iter().any(|exemption| exemption.case.strip_prefix(STATE).is_some_and(|name| name == format!("{state:?}"))));
    states
}

fn is_face_only(probe: &Probe) -> bool {
    probe.exempt.iter().any(|exemption| exemption.case == FACE_ONLY)
}

fn every_required_state_has_a_story(subject: &Subject) -> Result<(), String> {
    if is_face_only(subject.probe) {
        return if subject.stories.is_empty() {
            Ok(())
        } else {
            Err("the type declares `face-only` and has stories or a sample; a type with a sample runs through it, so drop `face-only`".to_owned())
        };
    }
    if subject.stories.is_empty() {
        return Err("no story shows the type; a type shown only inside another declares `#[component(sample = ..)]`, or `face-only` when it has no constructor".to_owned());
    }
    let missing: Vec<_> =
        required_states(subject.probe).into_iter().filter(|state| !subject.stories.iter().any(|story| story.covers.contains(state))).collect();
    if missing.is_empty() { Ok(()) } else { Err(format!("no story covers {missing:?}")) }
}

fn stories_show_and_hold_still(subject: &Subject) -> Result<(), String> {
    for story in &subject.stories {
        let mut fixture = Fixture::story(story);
        fixture.settle();
        let found = bypass::findings(&fixture);
        if !found.is_empty() {
            return Err(format!("{}: {found:?}", story.name));
        }
        let (size, id) = (fixture.shown().size(), fixture.root_id());
        fixture.idle(1);
        if matches!(subject.probe.shape, ShapeKind::Leaf) && !(size.x > 0.0 && size.y > 0.0) {
            return Err(format!("{}: a leaf story allocates nothing", story.name));
        }
        if fixture.root_id() != id {
            return Err(format!("{}: the id changed between frames", story.name));
        }
        if fixture.shown().size() != size {
            return Err(format!("{}: the size changed between settled frames", story.name));
        }
        if fixture.passes() != 1 {
            return Err(format!("{}: a settled frame ran {} passes", story.name, fixture.passes()));
        }
    }
    Ok(())
}

fn stories_pass_the_access_rules(subject: &Subject) -> Result<(), String> {
    for story in &subject.stories {
        let fixture = &subject.base(story).fixture;
        let found = bypass::findings(fixture);
        if !found.is_empty() {
            return Err(format!("{}: {found:?}", story.name));
        }
        let found = violations(fixture.ctx(), fixture.tree());
        if !found.is_empty() {
            return Err(format!("{}: {found:?}", story.name));
        }
    }
    Ok(())
}

fn focusable_stories_are_tab_reachable(subject: &Subject) -> Result<(), String> {
    for story in &subject.stories {
        let tree = subject.base(story).fixture.tree();
        let focusable = tree.keys().into_iter().any(|key| tree.declares_key(key, Declared::Focus) && is_interactive(tree, key));
        if !focusable {
            continue;
        }
        let mut fixture = super::drive::input_fixture(story);
        fixture.key(Key::Tab, Modifiers::NONE);
        if fixture.focused().is_none() {
            return Err(format!("{}: Tab reached nothing", story.name));
        }
    }
    Ok(())
}
