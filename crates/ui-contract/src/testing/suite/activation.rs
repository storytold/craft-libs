//! Activation, declared actions, AccessKit focus and cancelled presses.

use super::drive::{Plan, for_input, input_fixture, root_target, run, variant_name, variants};
use super::{ANY, Case, Subject};
use crate::contract::ShapeKind;
use crate::runtime::access::Role;
use crate::runtime::egui::access::{AccessTree, Declared, NodeKey};
use crate::runtime::egui::fixture::AccessActionKind;
use crate::testing::fixture::Fixture;
use crate::testing::pointer;
use crate::testing::story::{EditKind, Required, StoryData, StoryOutput};
use craft_behavior::Input;
use egui::{Id, Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(super) const CASES: &[Case] = &[
    Case::new("activate-once", ANY, each_activation_clicks_once),
    Case::new("part-activate", ANY, each_part_activates_once),
    Case::new("actions", ANY, declared_actions_are_handled),
    Case::new("intent-actions", ANY, every_intent_has_a_declared_action),
    Case::new("access-focus", ANY, access_focus_focuses),
    Case::new("press-cancel", ANY, escape_and_blur_cancel_a_press),
    Case::new("press-removal", ANY, removal_cancels_a_press),
];

/// Enter, Space, AccessKit `Click` and a primary click each give exactly one click on a component that reports
/// clicks and carries no value (value types are checked by the values case).
fn each_activation_clicks_once(subject: &Subject) -> Result<(), String> {
    if subject.probe.carries_value() {
        return Ok(());
    }
    for story in &subject.stories {
        let (fixture, plan) = subject.base(story).parts();
        let root = root_target(fixture);
        if !matches!(plan, Plan::Activate) || !fixture.tree().declares_key(root.node, Declared::Click) {
            continue;
        }
        for variant in variants(plan, subject.probe) {
            let mut fixture = input_fixture(story);
            fixture.take_outputs();
            run(&mut fixture, plan, variant);
            let clicks = fixture.take_outputs().iter().filter(|output| output.clicked).count();
            if clicks != 1 {
                return Err(format!("{}: {} gave {clicks} clicks", story.name, variant_name(plan, variant)));
            }
        }
    }
    Ok(())
}

/// Every part that senses `CLICK` of a component that carries no value gives exactly one click on each path it has:
/// a primary click, AccessKit `Click` when its node declares it, and Enter or Space after AccessKit `Focus` when it
/// is a focus stop; a collection's parts report events, not clicks, and are read by the collection cases. Two clicks mean the part's press also reached the root's activation.
fn each_part_activates_once(subject: &Subject) -> Result<(), String> {
    if subject.probe.carries_value()
        || matches!(subject.probe.input, Input::Text | Input::Editor | Input::Display)
        || matches!(subject.probe.shape, ShapeKind::Items)
    {
        return Ok(());
    }
    let keys = [(Key::Enter, subject.probe.keys.activate.enter()), (Key::Space, subject.probe.keys.activate.space())];
    for story in &subject.stories {
        let fixture = &subject.base(story).fixture;
        let Some(id) = fixture.root_id() else { continue };
        let mut parts = crate::runtime::pointer::sensing_parts(fixture.ctx(), id, craft_behavior::Sense::CLICK);
        parts.dedup();
        for part in parts {
            // Paths: 0 click, 1 AccessKit Click, 2 and 3 the activation keys.
            for path in 0..4 {
                let mut fixture = input_fixture(story);
                let Some(target) = fixture.root_id().and_then(|id| fixture.part(id, part)) else { continue };
                if fixture.tree().data_of(target.node).is_some_and(|node| node.is_disabled()) {
                    continue;
                }
                let declares = |fixture: &Fixture, action| fixture.tree().declares_key(target.node, action);
                let name = match path {
                    0 => "a click",
                    1 if declares(&fixture, Declared::Click) => "AccessKit Click",
                    2 if keys[0].1 && declares(&fixture, Declared::Focus) => "Enter",
                    3 if keys[1].1 && declares(&fixture, Declared::Focus) => "Space",
                    _ => continue,
                };
                fixture.take_outputs();
                match path {
                    0 => {
                        fixture.click(&target);
                    }
                    1 => {
                        fixture.access(AccessActionKind::Click, &target);
                    }
                    _ => {
                        fixture.access(AccessActionKind::Focus, &target);
                        fixture.settle();
                        fixture.key(keys[path - 2].0, Modifiers::NONE);
                    }
                }
                fixture.settle();
                let clicks = fixture.take_outputs().iter().filter(|output| output.clicked).count();
                if clicks != 1 {
                    return Err(format!("{}: {name} on the part {part:?} gave {clicks} clicks", story.name));
                }
            }
        }
    }
    Ok(())
}

/// What a node tree says about values and states: any change of it is an effect.
fn signature(tree: &AccessTree) -> Vec<String> {
    tree.keys()
        .into_iter()
        .map(|key| {
            tree.data_of(key).map_or_else(String::new, |node| {
                format!("{:?}|{:?}|{:?}|{:?}|{:?}", node.toggled(), node.is_expanded(), node.is_selected(), node.numeric_value(), node.value())
            })
        })
        .collect()
}

/// Every action a node declares changes something: a click, an edit, a node state or the focus.
fn declared_actions_are_handled(subject: &Subject) -> Result<(), String> {
    for story in subject.stories.iter().filter(|story| !story.covers.contains(&Required::Invalid)) {
        let fixture = &subject.base(story).fixture;
        let actions = [
            (Declared::Click, AccessActionKind::Click),
            (Declared::Increment, AccessActionKind::Increment),
            (Declared::Expand, AccessActionKind::Expand),
        ];
        let tree = fixture.tree();
        let nodes: Vec<_> = tree
            .keys()
            .into_iter()
            .filter(|key| tree.data_of(*key).is_some_and(|node| !node.is_disabled()))
            .flat_map(|key| actions.iter().filter(move |(declared, _)| tree.declares_key(key, *declared)).map(move |(_, kind)| (key, *kind)))
            .collect();
        for (key, kind) in nodes {
            let mut fixture = input_fixture(story);
            let Some(target) = fixture.target(key) else { continue };
            let (before, focus) = (signature(fixture.tree()), fixture.focused());
            fixture.take_outputs();
            fixture.access(kind, &target);
            fixture.settle();
            let outputs = fixture.take_outputs();
            let acted = outputs.iter().any(|output| output.clicked || !output.edits.is_empty())
                || signature(fixture.tree()) != before
                || fixture.focused() != focus;
            if !acted {
                return Err(format!("{}: {kind:?} on {:?} did nothing", story.name, fixture.tree().role_of(key)));
            }
        }
    }
    Ok(())
}

/// Whether any node of the tree reports itself expanded.
fn any_expanded(tree: &AccessTree) -> bool {
    tree.keys().into_iter().any(|key| tree.data_of(key).is_some_and(|node| node.is_expanded() == Some(true)))
}

/// Every intent the component takes has its AccessKit action declared on a node (core 12.2). The intents are read
/// from what the keys the type keeps do on a focused story: an activation key or arrow that gives a click or changes
/// the value needs `Click`; an arrow that changes the value of a control needs `Increment` or `Decrement`, declared
/// before the key (a step onto the end of a range offers none past it afterwards), or, on a
/// node that declares scroll actions on the arrow's axis (a view that pans), the scroll action of the arrow's
/// direction; an arrow that moves the checked or focused node of a group of `RADIO_BUTTON` nodes is a choice, not
/// a step, and needs `Click` declared on every enabled radio of the group instead; a key that opens something needs `Expand` and one that closes it needs `Collapse`, declared before the key. Text types take edit intents, which the text cases cover.
fn every_intent_has_a_declared_action(subject: &Subject) -> Result<(), String> {
    let probe = subject.probe;
    if matches!(probe.input, Input::Text | Input::Editor | Input::Display) {
        return Ok(());
    }
    let keys = [
        (Key::Enter, probe.keys.activate.enter(), None),
        (Key::Space, probe.keys.activate.space(), None),
        (Key::ArrowRight, true, Some(Declared::Increment)),
        (Key::ArrowUp, true, Some(Declared::Increment)),
        (Key::ArrowLeft, true, Some(Declared::Decrement)),
        (Key::ArrowDown, true, Some(Declared::Decrement)),
    ];
    // The scroll action each arrow is, and the pair of its axis.
    let scroll_of = |key: Key| match key {
        Key::ArrowRight => Some((Declared::ScrollRight, [Declared::ScrollLeft, Declared::ScrollRight])),
        Key::ArrowLeft => Some((Declared::ScrollLeft, [Declared::ScrollLeft, Declared::ScrollRight])),
        Key::ArrowDown => Some((Declared::ScrollDown, [Declared::ScrollUp, Declared::ScrollDown])),
        Key::ArrowUp => Some((Declared::ScrollUp, [Declared::ScrollUp, Declared::ScrollDown])),
        _ => None,
    };
    for story in subject.stories.iter().filter(|story| !story.covers.contains(&Required::Invalid)) {
        for &(key, _, step) in keys.iter().filter(|(_, kept, _)| *kept) {
            let mut fixture = input_fixture(story);
            fixture.key(Key::Tab, Modifiers::NONE);
            fixture.settle();
            let (before, was_open) = (fixture.data().snapshot(), any_expanded(fixture.tree()));
            let declares_before = |declared| {
                let tree = fixture.tree();
                tree.keys().into_iter().any(|node| tree.declares_key(node, declared))
            };
            let (can_expand, can_collapse) = (declares_before(Declared::Expand), declares_before(Declared::Collapse));
            let can_click = declares_before(Declared::Click);
            let could_step = step.is_some_and(declares_before);
            fixture.take_outputs();
            let focus_before = fixture.tree().focused_key();
            fixture.key(key, Modifiers::NONE);
            fixture.settle();
            let clicked = fixture.take_outputs().iter().any(|output| output.clicked);
            let changed = !fixture.data().changed_since(&before).is_empty();
            let radios = fixture.tree().with_role(Role::RADIO_BUTTON);
            let tree = fixture.tree();
            let roving =
                step.is_some() && radios.len() > 1 && radios.contains(&tree.focused_key()) && (changed || tree.focused_key() != focus_before);
            // A disabled radio takes no click, so it declares none.
            if roving && radios.iter().any(|&radio| !tree.is_disabled_key(radio) && !tree.declares_key(radio, Declared::Click)) {
                return Err(format!("{}: {key:?} moved the choice of a radio group and a radio declares no Click", story.name));
            }
            let mut needs = Vec::new();
            if clicked || (changed && step.is_none()) {
                needs.push(Declared::Click);
            }
            // A view that pans declares the scroll actions of its axis on its own node, and the arrow is one of them.
            let pans = crate::runtime::focus::focused_rec(fixture.ctx())
                .map(|rec| NodeKey::of(rec.stop.id))
                .zip(scroll_of(key))
                .filter(|(node, (_, axis))| axis.iter().any(|d| tree.declares_key(*node, *d)));
            if let Some(step) = step.filter(|_| changed && !roving && !matches!(probe.shape, ShapeKind::Items)) {
                needs.push(pans.map_or(step, |(_, (scroll, _))| scroll));
            }
            let opens = any_expanded(fixture.tree());
            for declared in needs {
                // A key that closes the container (Enter on its close action) takes the declaring node with it, so
                // `Click` counts when a node declared it before the key, as `Expand` and `Collapse` do.
                let before = (declared == Declared::Click && can_click) || (Some(declared) == step && could_step);
                if !before && !tree.keys().into_iter().any(|node| tree.declares_key(node, declared)) {
                    return Err(format!("{}: {key:?} took an intent and no node declares {declared:?}", story.name));
                }
            }
            if (!was_open && opens && !can_expand) || (was_open && !opens && !can_collapse) {
                let declared = if was_open { Declared::Collapse } else { Declared::Expand };
                return Err(format!("{}: {key:?} took an intent and no node declares {declared:?}", story.name));
            }
        }
    }
    Ok(())
}

/// AccessKit `Focus` on a node that declares it gives the component key focus.
fn access_focus_focuses(subject: &Subject) -> Result<(), String> {
    for story in &subject.stories {
        let mut fixture = input_fixture(story);
        let node =
            fixture.tree().keys().into_iter().find(|key| {
                fixture.tree().declares_key(*key, Declared::Focus) && fixture.tree().data_of(*key).is_some_and(|node| !node.is_disabled())
            });
        let Some(target) = node.and_then(|key| fixture.target(key)) else { continue };
        fixture.access(AccessActionKind::Focus, &target);
        fixture.settle();
        if fixture.focused().is_none() {
            return Err(format!("{}: AccessKit Focus focused nothing", story.name));
        }
    }
    Ok(())
}

/// Escape and the window losing focus during a press end it without a click or a commit.
fn escape_and_blur_cancel_a_press(subject: &Subject) -> Result<(), String> {
    for story in &subject.stories {
        let (fixture, plan) = subject.base(story).parts();
        if !matches!(plan, Plan::Activate | Plan::Drag(_)) {
            continue;
        }
        let root = root_target(fixture);
        if !fixture.tree().declares_key(root.node, Declared::Click) && !matches!(plan, Plan::Drag(_)) {
            continue;
        }
        for escape in [true, false] {
            let mut fixture = input_fixture(story);
            let target = root_target(&fixture);
            let before = fixture.data().snapshot();
            fixture.take_outputs();
            fixture.press(&target);
            if escape {
                fixture.key(Key::Escape, Modifiers::NONE);
            } else {
                fixture.frame(vec![pointer::window_blurred()]);
            }
            fixture.release(&target);
            fixture.settle();
            let outputs = fixture.take_outputs();
            let ended = outputs.iter().flat_map(|output| output.edits.iter()).any(|kind| matches!(kind, EditKind::Commit | EditKind::Set));
            if outputs.iter().any(|output| output.clicked) || ended || !fixture.data().changed_since(&before).is_empty() {
                return Err(format!("{}: a press cancelled by {} still acted", story.name, if escape { "Escape" } else { "blur" }));
            }
        }
    }
    Ok(())
}

/// A component that is not shown for one pass while a press on it is open loses the press: the pointer service
/// holds no capture afterwards, and the release that follows gives no click, commit or value change.
fn removal_cancels_a_press(subject: &Subject) -> Result<(), String> {
    for story in &subject.stories {
        let (fixture, plan) = subject.base(story).parts();
        let root = root_target(fixture);
        let part = match plan {
            Plan::Activate if fixture.tree().declares_key(root.node, Declared::Click) => None,
            Plan::Drag(grip) => Some(grip.part),
            _ => continue,
        };
        let data = Rc::new(RefCell::new(StoryData::default()));
        let outputs: Rc<RefCell<Vec<StoryOutput>>> = Rc::default();
        let (hidden, id): (Rc<Cell<bool>>, Rc<Cell<Option<Id>>>) = (Rc::default(), Rc::default());
        let (shared_data, shared_outputs, shared_hidden, shared_id, show) = (data.clone(), outputs.clone(), hidden.clone(), id.clone(), story.show);
        let fixture = Fixture::new(move |ui| {
            if shared_hidden.get() {
                return;
            }
            let output = show(ui, &mut shared_data.borrow_mut());
            shared_id.set(output.id);
            shared_outputs.borrow_mut().push(output);
        });
        let mut fixture = for_input(fixture);
        let Some(id) = id.get() else { continue };
        let Some(target) = part.and_then(|part| fixture.part(id, part)).or_else(|| fixture.node_of(id)) else { continue };
        let before = data.borrow().snapshot();
        outputs.borrow_mut().clear();
        fixture.press(&target);
        hidden.set(true);
        fixture.frame(Vec::new());
        hidden.set(false);
        if crate::testing::pointer::press_captured(fixture.ctx()) {
            return Err(format!("{}: a press still holds the pointer after its component was not shown for a pass", story.name));
        }
        fixture.release(&target);
        fixture.settle();
        let outputs = std::mem::take(&mut *outputs.borrow_mut());
        let ended = outputs.iter().flat_map(|output| output.edits.iter()).any(|kind| matches!(kind, EditKind::Commit | EditKind::Set));
        if outputs.iter().any(|output| output.clicked) || ended || !data.borrow().changed_since(&before).is_empty() {
            return Err(format!("{}: a press whose component was not shown for a pass still acted on release", story.name));
        }
    }
    Ok(())
}
