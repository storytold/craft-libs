//! The text nodes of AccessKit: a text component's value, actions and selection, and the `TextRun` nodes below it.

use egui::{Context, Id, Rect};
use std::ops::Range;

/// A selection end as AccessKit names it: a run node's key and a character index in the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NodeEnd {
    pub node: u64,
    pub character: usize,
}

/// A link inside a run: the characters it covers and where it leads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunLink {
    pub chars: Range<usize>,
    pub target: String,
}

/// One `TextRun` node.
pub(crate) struct RunNode<'a> {
    pub id: Id,
    pub parent: Id,
    pub rect: Rect,
    pub text: &'a str,
    pub rtl: bool,
    /// Each character's start offset from the run's left edge, and its advance.
    pub positions: &'a [f32],
    pub widths: &'a [f32],
    /// The links in the run, each written as a `Link` child of the run node.
    pub links: &'a [RunLink],
}

/// What a text component's own node says beyond the common facts.
pub(crate) struct TextNode<'a> {
    /// The text; `None` for a node that exposes none (a masked field).
    pub value: Option<&'a str>,
    pub editable: bool,
    /// The anchor and focus of the selection, `None` while the text has no runs.
    pub selection: Option<(Id, usize, Id, usize)>,
}

fn lengths(text: &str) -> Vec<u8> {
    text.chars().map(|c| u8::try_from(c.len_utf8()).unwrap_or(u8::MAX)).collect()
}

/// The character index each word starts at.
fn word_starts(text: &str) -> Vec<u8> {
    let mut starts = Vec::new();
    let mut previous_space = true;
    for (at, c) in text.chars().enumerate() {
        if previous_space && !c.is_whitespace() {
            starts.push(u8::try_from(at).unwrap_or(u8::MAX));
        }
        previous_space = c.is_whitespace();
    }
    starts
}

/// Creates and fills the `TextRun` node of `run` below its parent.
pub(crate) fn write_run(ctx: &Context, run: &RunNode<'_>) {
    super::register_parent(ctx, run.id, run.parent);
    crate::runtime::egui::widgets::note_node(ctx, run.id);
    ctx.accesskit_node_builder(run.id, |node| {
        node.set_role(accesskit::Role::TextRun);
        node.set_bounds(accesskit::Rect {
            x0: f64::from(run.rect.min.x),
            y0: f64::from(run.rect.min.y),
            x1: f64::from(run.rect.max.x),
            y1: f64::from(run.rect.max.y),
        });
        node.set_value(run.text.to_owned());
        node.set_character_lengths(lengths(run.text));
        node.set_character_positions(run.positions.to_vec());
        node.set_character_widths(run.widths.to_vec());
        node.set_word_starts(word_starts(run.text));
        node.set_text_direction(if run.rtl { accesskit::TextDirection::RightToLeft } else { accesskit::TextDirection::LeftToRight });
    });
    for (at, link) in run.links.iter().enumerate() {
        write_link(ctx, run, at, link);
    }
}

/// The `Link` child of a run for `link`: named by the characters it covers, bounded by their extent in the run.
fn write_link(ctx: &Context, run: &RunNode<'_>, at: usize, link: &RunLink) {
    let end = link.chars.end.min(run.positions.len()).min(run.widths.len());
    let start = link.chars.start.min(end);
    if start == end {
        return;
    }
    let name: String = run.text.chars().skip(start).take(end - start).collect();
    let (from, to) = (run.positions[start], run.positions[end - 1] + run.widths[end - 1]);
    let id = run.id.with(("link", at));
    super::register_parent(ctx, id, run.id);
    crate::runtime::egui::widgets::note_node(ctx, id);
    ctx.accesskit_node_builder(id, |node| {
        node.set_role(accesskit::Role::Link);
        node.set_bounds(accesskit::Rect {
            x0: f64::from(run.rect.min.x + from.min(to)),
            y0: f64::from(run.rect.min.y),
            x1: f64::from(run.rect.min.x + from.max(to)),
            y1: f64::from(run.rect.max.y),
        });
        node.set_label(name);
        node.set_url(link.target.clone());
    });
}

/// Adds the text value, the text actions and the selection to the node of `id`.
pub(crate) fn write_text(ctx: &Context, id: Id, text: &TextNode<'_>) {
    ctx.accesskit_node_builder(id, |node| {
        if let Some(value) = text.value {
            node.set_value(value.to_owned());
        }
        node.add_action(accesskit::Action::SetTextSelection);
        if text.editable {
            node.add_action(accesskit::Action::SetValue);
            node.add_action(accesskit::Action::ReplaceSelectedText);
        }
        if let Some((anchor, anchor_at, focus, focus_at)) = text.selection {
            node.set_text_selection(accesskit::TextSelection {
                anchor: accesskit::TextPosition { node: anchor.accesskit_id(), character_index: anchor_at },
                focus: accesskit::TextPosition { node: focus.accesskit_id(), character_index: focus_at },
            });
        }
    });
}

/// Folds the node egui gives the text widget `widget` into the node of `owner`, which carries the text, its actions
/// and its runs: the widget's node is hidden, a password or multi-line widget refines the role of an owner that is a
/// plain text input, and the owner's node stands for the widget's key focus.
pub(crate) fn fold_widget(ctx: &Context, widget: Id, owner: Id) {
    let kind = ctx.accesskit_node_builder(widget, |node| node.role());
    if let Some(kind @ (accesskit::Role::PasswordInput | accesskit::Role::MultilineTextInput)) = kind {
        ctx.accesskit_node_builder(owner, |node| {
            if node.role() == accesskit::Role::TextInput {
                node.set_role(kind);
            }
        });
    }
    super::hide(ctx, widget);
    super::note_folded(ctx, widget, owner);
    crate::runtime::access::fold::request_text(ctx, widget, owner);
}

/// Sets the text selection of the node of `id` between two run nodes, for a node whose runs a content painter
/// describes.
pub(crate) fn write_selection(ctx: &Context, id: Id, anchor: (Id, usize), focus: (Id, usize)) {
    ctx.accesskit_node_builder(id, |node| {
        node.set_text_selection(accesskit::TextSelection {
            anchor: accesskit::TextPosition { node: anchor.0.accesskit_id(), character_index: anchor.1 },
            focus: accesskit::TextPosition { node: focus.0.accesskit_id(), character_index: focus.1 },
        });
    });
}

/// The selection a `SetTextSelection` request carries.
pub(crate) fn requested_selection(request: &craft_behavior::AccessRequest) -> Option<(NodeEnd, NodeEnd)> {
    if request.action != accesskit::Action::SetTextSelection {
        return None;
    }
    let Some(accesskit::ActionData::SetTextSelection(selection)) = &request.data else { return None };
    let end = |position: accesskit::TextPosition| NodeEnd { node: position.node.0, character: position.character_index };
    Some((end(selection.anchor), end(selection.focus)))
}

/// The text a `SetValue` request carries.
pub(crate) fn requested_value(request: &craft_behavior::AccessRequest) -> Option<String> {
    if request.action != accesskit::Action::SetValue {
        return None;
    }
    match &request.data {
        Some(accesskit::ActionData::Value(text)) => Some(text.to_string()),
        _ => None,
    }
}

/// Whether `request` is one the text adapter serves.
pub(crate) fn is_text_request(request: &craft_behavior::AccessRequest) -> bool {
    requested_selection(request).is_some() || requested_value(request).is_some()
}
