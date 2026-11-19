//! The container rows of the 22.4 table: title naming, bodies on `Mount::Layer` (Escape, focus trap, focus return,
//! placement memory) and scrolling. Each case runs over the stories of a type of the `Body` shape and looks only at
//! what the story shows: a layer case skips the stories whose container is not on a layer, a scroll case those whose
//! body does not overflow.

use super::Subject;
use super::drive::input_fixture;
use super::nodes::{focus_within, root, subtree};
use crate::contract::{MOVE_BAND, ShapeKind};
use crate::runtime::access::Role;
use crate::runtime::egui::access::NodeKey;
use crate::runtime::overlay;
use crate::testing::access::is_interactive;
use crate::testing::fixture::Fixture;
use crate::testing::story::Story;
use egui::{Key, Modifiers, Vec2};

fn is_body(subject: &Subject) -> bool {
    matches!(subject.probe.shape, ShapeKind::Body)
}

/// Whether the story's container is a layer of the overlay host that is not fading out.
fn on_layer(fixture: &Fixture) -> bool {
    fixture.root_id().is_some_and(|id| overlay::entry(fixture.ctx(), id).is_some_and(|entry| !entry.exiting))
}

/// A fixture of `story` settled for input, when its container is on a layer.
fn layer_fixture(story: &Story) -> Option<Fixture> {
    let fixture = input_fixture(story);
    on_layer(&fixture).then_some(fixture)
}

/// Whether `container` is a field container: exactly one control inside it is labelled, by nodes inside the container,
/// and every other interactive node inside is a part of that control: below it, or beside it under the same parent
/// and named like it or drawn inside it (a spin button and its text input, a scroll bar in a text area). A button beside
/// the control that is neither is no part of it. The label names the control, so the group needs no name.
fn is_field_of_one_control(tree: &crate::testing::access::AccessTree, container: crate::testing::access::NodeKey) -> bool {
    let inside = subtree(tree, container);
    let interactive: Vec<_> = inside.iter().copied().filter(|key| crate::testing::access::is_interactive(tree, *key)).collect();
    let labelled: Vec<_> = interactive.iter().copied().filter(|key| !tree.labelled_by_key(*key).is_empty()).collect();
    let [control] = labelled.as_slice() else { return false };
    let parts = subtree(tree, *control);
    let bounds = tree.bounds_of(*control);
    let is_part = |key: &NodeKey| {
        parts.contains(key)
            || (tree.parent_of_key(*key) == tree.parent_of_key(*control)
                && (tree.name_of(*key).is_some() && tree.name_of(*key) == tree.name_of(*control)
                    || bounds.zip(tree.bounds_of(*key)).is_some_and(|(control, part)| control.contains_rect(part))))
    };
    tree.labelled_by_key(*control).iter().all(|label| inside.contains(label)) && interactive.iter().all(is_part)
}

/// A container that is a dialog or a group has a role that needs a name: the title names it. A field container's label
/// names its one control instead, which then carries the name and the `labelled_by` link, and the group has none.
pub(super) fn title_names_the_container(subject: &Subject) -> Result<(), String> {
    if !is_body(subject) {
        return Ok(());
    }
    subject.stories.iter().try_for_each(|story| title_from(story, &subject.base(story).fixture))
}

/// The title case for one story.
pub fn title_case(story: &Story) -> Result<(), String> {
    let mut fixture = Fixture::story(story);
    fixture.settle();
    title_from(story, &fixture)
}

/// The title case for `story`, settled in `fixture`.
fn title_from(story: &Story, fixture: &Fixture) -> Result<(), String> {
    let Some(container) = root(fixture) else { return Ok(()) };
    let role = fixture.tree().role_of(container.node);
    let tree = fixture.tree();
    if (role == Some(Role::DIALOG) || role == Some(Role::GROUP))
        && crate::testing::access::name(tree, container.node).is_none()
        && !(role == Some(Role::GROUP) && is_field_of_one_control(tree, container.node))
    {
        return Err(format!("{}: the container's node has the role {role:?} and no name", story.name));
    }
    Ok(())
}

/// Escape closes the layer: no layer that takes keys is left on top of it. A layer that takes no keys (modal chrome, a
/// toast) and a layer whose scope ignores Escape (a blocking overlay nobody can cancel, or a pane such as a floating
/// panel) are not asked to.
pub(super) fn layer_closes_on_escape(subject: &Subject) -> Result<(), String> {
    if !is_body(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some(mut fixture) = layer_fixture(story) else { continue };
        let Some(id) = fixture.root_id() else { continue };
        let takes_keys = overlay::entry(fixture.ctx(), id).is_some_and(|entry| entry.takes_keys());
        let ignores = crate::runtime::focus::with(fixture.ctx(), |state| {
            state.last_scopes.get(&id).is_some_and(|scope| scope.policy.cancel == craft_behavior::focus::CancelRule::Ignore)
        });
        if !takes_keys || ignores {
            continue;
        }
        fixture.key(Key::Escape, Modifiers::NONE);
        fixture.settle();
        if on_layer(&fixture) {
            return Err(format!("{}: the layer is still open after Escape", story.name));
        }
        if overlay::top(fixture.ctx()) == Some(id) {
            return Err(format!("{}: the closed layer still receives Cancel", story.name));
        }
    }
    Ok(())
}

/// Tab and Shift+Tab stay inside the layer: a trap, since a layer owns its focus scope. A scope that does not trap
/// (a dropdown, whose focus stays on its opener) is not asked to.
pub(super) fn layer_traps_focus(subject: &Subject) -> Result<(), String> {
    if !is_body(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some(mut fixture) = layer_fixture(story) else { continue };
        let Some(id) = fixture.root_id() else { continue };
        let traps = crate::runtime::focus::with(fixture.ctx(), |state| state.last_scopes.get(&id).is_some_and(|scope| scope.policy.trap));
        if !traps {
            continue;
        }
        let Some(container) = root(&fixture) else { continue };
        let stops = subtree(fixture.tree(), container.node).into_iter().filter(|key| is_interactive(fixture.tree(), *key)).count();
        if stops == 0 {
            continue;
        }
        for step in 0..stops + 2 {
            fixture.key(Key::Tab, Modifiers::NONE);
            if !focus_within(&fixture, container.node) {
                return Err(format!("{}: Tab number {} moved focus out of the layer", story.name, step + 1));
            }
        }
        fixture.key(Key::Tab, Modifiers::SHIFT);
        if !focus_within(&fixture, container.node) {
            return Err(format!("{}: Shift+Tab moved focus out of the layer", story.name));
        }
    }
    Ok(())
}

/// After Escape closes the layer, the AccessKit focus is not on a node of the closed container.
pub(super) fn layer_leaves_no_dangling_focus(subject: &Subject) -> Result<(), String> {
    if !is_body(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some(mut fixture) = layer_fixture(story) else { continue };
        let Some(container) = root(&fixture) else { continue };
        let was: Vec<NodeKey> = subtree(fixture.tree(), container.node);
        fixture.key(Key::Escape, Modifiers::NONE);
        fixture.settle();
        if was.contains(&fixture.tree().focused_key()) {
            return Err(format!("{}: focus stays on a node of the closed layer", story.name));
        }
    }
    Ok(())
}

/// A layer with a move band follows a drag of the band and keeps the place it was dragged to.
pub(super) fn layer_remembers_its_placement(subject: &Subject) -> Result<(), String> {
    if !is_body(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let Some(mut fixture) = layer_fixture(story) else { continue };
        let Some(id) = fixture.root_id() else { continue };
        let Some(band) = fixture.part(id, MOVE_BAND) else { continue };
        let Some(before) = root(&fixture).map(|container| container.rect) else { continue };
        fixture.drag(&band, Vec2::new(40.0, 30.0));
        fixture.settle();
        let Some(moved) = root(&fixture).map(|container| container.rect) else { continue };
        if !(moved.min.x > before.min.x || moved.min.y > before.min.y) {
            return Err(format!("{}: a drag of the move band left the layer at {before:?}", story.name));
        }
        fixture.idle(3);
        let Some(later) = root(&fixture).map(|container| container.rect) else { continue };
        if later.min != moved.min {
            return Err(format!("{}: the layer went from {moved:?} back to {later:?} with no input", story.name));
        }
    }
    Ok(())
}

/// How far below the container's bottom edge its lowest node reaches.
fn overflow(fixture: &Fixture, container: NodeKey, bottom: f32) -> f32 {
    subtree(fixture.tree(), container)
        .into_iter()
        .filter_map(|key| fixture.tree().bounds_of(key))
        .map(|bounds| bounds.max.y - bottom)
        .fold(0.0, f32::max)
}

/// A body whose content overflows the container scrolls to its end under the wheel, with the pointer over it.
pub(super) fn overflowing_body_scrolls_to_its_end(subject: &Subject) -> Result<(), String> {
    if !is_body(subject) {
        return Ok(());
    }
    for story in &subject.stories {
        let mut fixture = input_fixture(story);
        let Some(container) = root(&fixture) else { continue };
        let start = overflow(&fixture, container.node, container.rect.max.y);
        if start <= 1.0 {
            continue;
        }
        let at = container.rect.center();
        fixture.frame(vec![crate::testing::pointer::moved(at)]);
        for _ in 0..40 {
            fixture.frame(vec![crate::testing::pointer::wheel(-120.0)]);
        }
        fixture.settle();
        let Some(after) = root(&fixture) else { continue };
        let left = overflow(&fixture, after.node, after.rect.max.y);
        if left > 1.0 {
            return Err(format!("{}: after scrolling to the end the content still reaches {left} points below the container", story.name));
        }
    }
    Ok(())
}
