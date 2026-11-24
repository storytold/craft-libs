//! Writes the label-scope and merge patches `runtime::access::scope` decides, and the status node of announcements.

use super::Toggle;
use crate::runtime::access::scope::{LabelPatch, MergePatch};
use egui::{Context, Id};

/// Writes one label-scope change.
pub(crate) fn write_label_patch(ctx: &Context, patch: &LabelPatch) {
    match patch {
        LabelPatch::Label { id, text, disabled } => {
            ctx.accesskit_node_builder(*id, |node| {
                // The label's leaf wrote its node, named, in this pass; one it did not write is made here, named by the
                // same rule.
                if node.role() == accesskit::Role::Unknown {
                    node.set_role(accesskit::Role::Label);
                    super::node::write_name(node, text);
                }
                if *disabled {
                    node.set_disabled();
                }
            });
        }
        LabelPatch::DescribedBy { node, label } => {
            ctx.accesskit_node_builder(*node, |node| {
                let mut described = node.described_by().to_vec();
                described.push(label.accesskit_id());
                node.set_described_by(described);
            });
        }
        LabelPatch::LabelledBy { node, label, text } => {
            ctx.accesskit_node_builder(*node, |node| {
                node.set_label(text.clone());
                if let Some(label) = label {
                    node.set_labelled_by(vec![label.accesskit_id()]);
                }
            });
        }
        LabelPatch::GroupLabelledBy { container, label } => {
            ctx.accesskit_node_builder(*container, |node| node.set_labelled_by(vec![label.accesskit_id()]));
        }
        LabelPatch::Detail { node, text } => {
            ctx.accesskit_node_builder(*node, |node| {
                let joined = match node.description() {
                    Some(own) if !own.is_empty() => format!("{own}. {text}"),
                    _ => text.clone(),
                };
                node.set_description(joined);
            });
        }
    }
}

/// Writes a merged container's name, description and states into its child's node.
pub(crate) fn write_merge_patch(ctx: &Context, patch: &MergePatch) {
    ctx.accesskit_node_builder(patch.child, |node| {
        if let Some(name) = &patch.name {
            node.set_label(name.clone());
        }
        if let Some(description) = &patch.description {
            node.set_description(description.clone());
        }
        if patch.disabled {
            node.set_disabled();
        }
        if patch.selected {
            node.set_selected(true);
        }
        match patch.toggled {
            Some(Toggle::Mixed) => node.set_toggled(accesskit::Toggled::Mixed),
            Some(Toggle::On) => node.set_toggled(accesskit::Toggled::True),
            Some(Toggle::Off) => node.set_toggled(accesskit::Toggled::False),
            None => {}
        }
        if patch.invalid {
            node.set_invalid(accesskit::Invalid::True);
        }
        if patch.busy {
            node.set_busy();
        }
    });
}

/// Writes the place in its set of the node `id`, which the node's own report left to its roving group.
pub(crate) fn write_set_position(ctx: &Context, id: Id, index: usize, size: Option<usize>) {
    ctx.accesskit_node_builder(id, |node| {
        node.set_position_in_set(index);
        match size {
            Some(size) => node.set_size_of_set(size),
            None => node.clear_size_of_set(),
        }
    });
}

/// The status node `id`: one live node holding `text`, none while there is nothing to say.
pub(crate) fn write_status(ctx: &Context, id: Id, text: &str, assertive: bool) {
    crate::runtime::egui::widgets::note_node(ctx, id);
    ctx.accesskit_node_builder(id, |node| {
        node.set_role(accesskit::Role::Status);
        node.set_live(if assertive { accesskit::Live::Assertive } else { accesskit::Live::Polite });
        node.set_live_atomic();
        if !text.is_empty() {
            node.set_label(text);
        }
    });
}
