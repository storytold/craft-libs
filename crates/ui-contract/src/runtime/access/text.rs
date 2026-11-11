//! Text accessibility (12.3, 18.4): the value, the text actions, the selection and one `TextRun` node per run of
//! each visible line, for a text part and for an editor that draws its own caret. The facts come from the
//! component's layout; the policy (what is exposed, what a password hides, how a request maps back to a position)
//! is here, and `runtime::egui::access` writes the nodes.

use crate::runtime::egui::access::{self as adapter, NodeEnd, RunNode, TextNode};
use crate::runtime::egui::memory::Service;
use crate::runtime::keys;
use crate::runtime::memory::sub;
use craft_behavior::{AccessRequest, PartKey, PartRef, TextPos};
use egui::{Context, Id, Rect};
use std::collections::HashMap;
use std::ops::Range;

/// The part key of a run node, indexed in document order.
const RUN: PartKey = PartKey::new("text_run");

/// One bidi run of a line, owned.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub bytes: Range<usize>,
    pub rtl: bool,
    /// Each character's start offset from the line's left edge, and its advance.
    pub positions: Vec<f32>,
    pub widths: Vec<f32>,
}

/// One visible line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub text: String,
    pub rect: Rect,
    /// At least one run: an empty line has one empty run, which holds the caret.
    pub runs: Vec<Run>,
}

/// What a text component tells AccessKit about its text this pass.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextFacts {
    /// The egui widget that holds the text and key focus; its node folds into the owner's.
    pub widget: Id,
    /// The whole text, for the node's value (not exposed while masked).
    pub text: String,
    /// A password: the node carries no value and exposes no runs.
    pub masked: bool,
    pub editable: bool,
    pub lines: Vec<Line>,
    /// The anchor and focus as a line index and a byte offset in the line.
    pub selection: Option<(TextPos, TextPos)>,
}

/// A run node as the last report laid it out, for mapping a request back to a position.
struct Entry {
    node: u64,
    line: usize,
    bytes: Range<usize>,
    text: String,
}

#[derive(Default)]
struct Tables(HashMap<Id, Vec<Entry>>);

pub(crate) fn run_id(owner: Id, line: usize, run: usize) -> Id {
    sub(owner, ("text_run", line, run))
}

fn end_of(entries: &[Entry], owner: Id, pos: TextPos) -> Option<(Id, usize)> {
    let mut on_line = entries.iter().enumerate().filter(|(_, entry)| entry.line == pos.line).peekable();
    let mut found = None;
    while let Some((at, entry)) = on_line.next() {
        found = Some((at, entry));
        if pos.byte < entry.bytes.end || on_line.peek().is_none() {
            break;
        }
    }
    let (at, entry) = found?;
    let within = pos.byte.saturating_sub(entry.bytes.start).min(entry.text.len());
    let character = entry.text.get(..within).map_or(entry.text.chars().count(), |head| head.chars().count());
    let run = entries[..at].iter().filter(|other| other.line == entry.line).count();
    Some((run_id(owner, entry.line, run), character))
}

/// Whether the text part's `widget` has an AccessKit node this pass: the facts are built only then.
pub(crate) fn exposed(ctx: &Context, widget: Id) -> bool {
    adapter::enabled(ctx, widget)
}

/// Adds the text facts to the node of `owner` and fills its run nodes. The owner's node is the one node of the text:
/// the widget's own node folds into it, so a component whose root edits its value through the text (a text field, a
/// spin button, an editable combo box) is one node that takes focus and the text actions. Call it after the node's
/// common facts.
pub(crate) fn report_text(ctx: &Context, owner: Id, facts: &TextFacts) {
    adapter::fold_widget(ctx, facts.widget, owner);
    let mut entries = Vec::new();
    if !facts.masked {
        for (line_at, line) in facts.lines.iter().enumerate() {
            for (run_at, run) in line.runs.iter().enumerate() {
                let id = run_id(owner, line_at, run_at);
                let text = line.text.get(run.bytes.clone()).unwrap_or_default();
                adapter::write_run(
                    ctx,
                    &RunNode { id, parent: owner, rect: line.rect, text, rtl: run.rtl, positions: &run.positions, widths: &run.widths, links: &[] },
                );
                let index = u64::try_from(entries.len()).ok();
                keys::register_node(ctx, id, owner, Some(PartRef { key: RUN, index }), false);
                entries.push(Entry { node: adapter::node_key(id), line: line_at, bytes: run.bytes.clone(), text: text.to_owned() });
            }
        }
    }
    let selection = facts.selection.and_then(|(anchor, focus)| {
        let (anchor_node, anchor_at) = end_of(&entries, owner, anchor)?;
        let (focus_node, focus_at) = end_of(&entries, owner, focus)?;
        Some((anchor_node, anchor_at, focus_node, focus_at))
    });
    let value = (!facts.masked).then_some(facts.text.as_str());
    adapter::write_text(ctx, owner, &TextNode { value, editable: facts.editable, selection });
    Service::<Tables>::run(ctx, |tables| tables.0.insert(owner, entries));
}

/// Whether `request` is a text request the text adapter serves: `SetValue` with text or `SetTextSelection`.
pub(crate) fn is_text_request(request: &AccessRequest) -> bool {
    adapter::is_text_request(request)
}

/// The text a `SetValue` request carries.
pub(crate) fn requested_value(request: &AccessRequest) -> Option<String> {
    adapter::requested_value(request)
}

/// The selection a `SetTextSelection` request names, as line and byte positions of the last report.
pub(crate) fn requested_selection(ctx: &Context, owner: Id, request: &AccessRequest) -> Option<(TextPos, TextPos)> {
    let (anchor, focus) = adapter::requested_selection(request)?;
    Service::<Tables>::run(ctx, |tables| {
        let entries = tables.0.get(&owner)?;
        let pos = |end: NodeEnd| {
            let entry = entries.iter().find(|entry| entry.node == end.node)?;
            let within = entry.text.char_indices().nth(end.character).map_or(entry.text.len(), |(byte, _)| byte);
            Some(TextPos::new(entry.line, entry.bytes.start + within))
        };
        Some((pos(anchor)?, pos(focus)?))
    })
}

/// The run and character index a position maps to among runs given as `(line, bytes, text)`, for trials.
#[cfg(feature = "testing")]
pub(crate) fn end_of_runs(owner: Id, runs: &[(usize, Range<usize>, &str)], pos: TextPos) -> Option<(Id, usize)> {
    let entries: Vec<Entry> = runs
        .iter()
        .enumerate()
        .map(|(at, (line, bytes, text))| Entry { node: at as u64, line: *line, bytes: bytes.clone(), text: (*text).to_owned() })
        .collect();
    end_of(&entries, owner, pos)
}
