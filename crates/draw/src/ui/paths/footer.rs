//! The footer of the Paths panel.

use std::fmt::Debug;
use std::hash::Hash;

use craft_behavior::IconId;
use craft_ui::primitives::IconSpec;
use craft_ui::sections::{FooterAction, FooterBar};
use egui::{Id, Ui};

use super::{PathKind, PathRow, PathsEvent, PathsWords};

/// A button of the Paths footer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PathsFooterButton {
    /// Fill the path with the foreground colour; disabled while the document cannot be filled.
    Fill,
    /// Stroke the path.
    Stroke,
    /// Load the path as a selection; a row dropped on it is loaded.
    Load,
    /// Make a work path from the selection.
    WorkPath,
    /// Add a vector mask from the path to the layer.
    VectorMask,
    /// Create a new path; a row dropped on it is duplicated.
    New,
    /// Delete the path; a row dropped on it is deleted.
    Delete,
}

/// What a row drag carries for the footer's Delete button: the list it started in, the row the pointer grabbed and
/// that row's kind.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PathDrag<K> {
    pub(super) list: Id,
    pub(super) grabbed: K,
    pub(super) kind: PathKind,
}

/// The buttons from the end edge: the footer lays its actions out from there, so this is the reverse of the reading
/// order Fill, Stroke, Load, Work path, Vector mask, New, Delete.
const FROM_END: [PathsFooterButton; 7] = [
    PathsFooterButton::Delete,
    PathsFooterButton::New,
    PathsFooterButton::VectorMask,
    PathsFooterButton::WorkPath,
    PathsFooterButton::Load,
    PathsFooterButton::Stroke,
    PathsFooterButton::Fill,
];

fn icon_name(button: PathsFooterButton) -> &'static str {
    match button {
        PathsFooterButton::Fill => "paint_bucket",
        PathsFooterButton::Stroke => "circle",
        PathsFooterButton::Load => "ellipse_dashed",
        PathsFooterButton::WorkPath => "spline",
        PathsFooterButton::VectorMask => "layer_mask",
        PathsFooterButton::New => "plus",
        PathsFooterButton::Delete => "trash",
    }
}

/// Shows the footer and returns its events in order.
///
/// - A click reports [`PathsEvent::Footer`] with the selected rows and the modifiers held.
/// - Delete takes a dragged row of the panel's list whose kind drags: a drop reports [`PathsEvent::FooterDrop`] with
///   the row the pointer grabbed. Delete's tooltip names the vector mask while the selected row is one.
pub(super) fn show_footer<K: Clone + Eq + Hash + Debug + Send + Sync + 'static>(
    ui: &mut Ui,
    rows: &[PathRow<K>],
    words: &PathsWords,
    list: Id,
) -> Vec<PathsEvent<K>> {
    let selected: Vec<&PathRow<K>> = rows.iter().filter(|row| row.selected).collect();
    let on_mask = selected.first().is_some_and(|row| row.kind == PathKind::VectorMask);
    let actions = FROM_END.map(|button| {
        let tip = match button {
            PathsFooterButton::Fill => words.fill_path.clone(),
            PathsFooterButton::Stroke => words.stroke_path.clone(),
            PathsFooterButton::Load => words.load_selection.clone(),
            PathsFooterButton::WorkPath => words.make_work_path.clone(),
            PathsFooterButton::VectorMask => words.vector_mask.clone(),
            PathsFooterButton::New => words.new_path.clone(),
            PathsFooterButton::Delete if on_mask => words.delete_vector_mask.clone(),
            PathsFooterButton::Delete => words.delete_path.clone(),
        };
        let action = FooterAction::new(IconSpec::svg(IconId::new(icon_name(button))), tip);
        if button == PathsFooterButton::Delete {
            action.accepts_drops(move |drag: &PathDrag<K>| drag.list == list && drag.kind.drags())
        } else {
            action
        }
    });
    let out = FooterBar::actions(actions).show_footer(ui);
    let mut events = Vec::new();
    if let Some(button) = out.picked.and_then(|at| FROM_END.get(at).copied()) {
        events.push(PathsEvent::footer(button, selected.iter().map(|row| row.key.clone()).collect(), out.mods));
    }
    if let Some(dropped) = &out.dropped
        && let (Some(button), Some(drag)) = (FROM_END.get(dropped.index).copied(), dropped.payload::<PathDrag<K>>())
    {
        events.push(PathsEvent::footer_drop(button, vec![drag.grabbed.clone()]));
    }
    events
}
