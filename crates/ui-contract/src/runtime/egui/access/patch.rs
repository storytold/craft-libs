//! Writes the label-scope and merge patches `runtime::access::scope` decides, and the status node of announcements.

use super::Toggle;
use crate::runtime::access::scope::{LabelPatch, MergePatch};
use egui::{Context, Id};

/// Writes one label-scope change.
pub(crate) fn write_label_patch(ctx: &Context, patch: &LabelPatch) {
    match patch {
        LabelPatch::Label { id, text, disabled } => {
            ctx.accesskit_node_builder(*id, |node| {
                if node.role() == accesskit::Role::Unknown {
                    node.set_role(accesskit::Role::Label);
                }
                if node.label().is_none() {
                    node.set_label(text.clone());
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
                node.set_labelled_by(vec![label.accesskit_id()]);
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
