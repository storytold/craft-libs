//! Decision records and findings. Inspected regions ([`record`]) collect pipeline steps. Fixture frames also collect
//! contract findings. The region's record is what the debug inspector draws: every component shown in it
//! with its box and access facts, the hit areas the sensing core registered and the focus stops in Tab order.

use crate::runtime::egui::areas;
use crate::runtime::egui::areas::Layer;
use crate::runtime::egui::sizing::{Rerun, request_discard};
use crate::runtime::reads::{self, BindKey};
use crate::runtime::{focus, measure, pass_state, pointer};
use craft_behavior::PartRef;
use craft_keymap::CommandRef;
use egui::{Context, Id, IdMap, IdSet, Rect, Ui};
use std::sync::Arc;

/// A rule a component broke that the pipeline reports instead of panicking.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Finding {
    /// The rule's name.
    pub rule: &'static str,
    /// The component's type.
    pub component: &'static str,
    /// What the finding is about, such as a part or an id.
    pub subject: String,
}

#[derive(Default)]
pub(crate) struct Log {
    steps: IdMap<Vec<&'static str>>,
    findings: Vec<Finding>,
    canvas_controls: Vec<&'static str>,
    /// The inspected regions open in the running pass, innermost last.
    open: Vec<Open>,
    /// The latest record of each region, and the element its list points at, by the region's key.
    kept: IdMap<Kept>,
}

/// Whether an explicit inspector region is collecting elements and their pipeline steps.
pub(crate) fn recording(ctx: &Context) -> bool {
    pass_state::with::<Log, _>(ctx, |log| !log.open.is_empty())
}

#[cfg(feature = "testing")]
use crate::runtime::egui::fixture::driving as fixture_frame;

#[cfg(not(feature = "testing"))]
fn fixture_frame() -> bool {
    false
}

/// Starts the step record of `id` for a new show.
pub(crate) fn begin(ctx: &Context, id: Id) {
    pass_state::with::<Log, _>(ctx, |log| log.steps.insert(id, Vec::new()));
}

/// Records that `step` ran for `id`.
pub(crate) fn step(ctx: &Context, id: Id, step: &'static str) {
    pass_state::with::<Log, _>(ctx, |log| log.steps.entry(id).or_default().push(step));
}

/// The steps the latest show of `id` ran, in order.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "read by trials and the debug inspector"))]
pub(crate) fn steps_of(ctx: &Context, id: Id) -> Vec<&'static str> {
    pass_state::with::<Log, _>(ctx, |log| log.steps.get(&id).cloned().unwrap_or_default())
}

/// Reports a finding.
pub(crate) fn finding(ctx: &Context, finding: Finding) {
    if !fixture_frame() && !recording(ctx) {
        return;
    }
    pass_state::with::<Log, _>(ctx, |log| {
        if !log.findings.contains(&finding) {
            log.findings.push(finding);
        }
    });
}

/// Every finding reported on the context.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "read by trials and the debug inspector"))]
pub(crate) fn findings(ctx: &Context) -> Vec<Finding> {
    pass_state::with::<Log, _>(ctx, |log| log.findings.clone())
}

/// Records that the canvas control `type_name` showed in this pass, so the canvas-control cases can check that a
/// press away from its parts reaches the tool.
pub(crate) fn canvas_control(ctx: &Context, type_name: &'static str) {
    if !fixture_frame() && !recording(ctx) {
        return;
    }
    pass_state::with::<Log, _>(ctx, |log| {
        if !log.canvas_controls.contains(&type_name) {
            log.canvas_controls.push(type_name);
        }
    });
}

/// The canvas-control types that have shown on the context.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "read by trials and the debug inspector"))]
pub(crate) fn canvas_controls(ctx: &Context) -> Vec<&'static str> {
    pass_state::with::<Log, _>(ctx, |log| log.canvas_controls.clone())
}

/// What a node says, as step 16 reported it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Facts {
    /// The AccessKit role's name.
    pub role: String,
    /// The accessible name: its own, else the names of the nodes of the region that label it.
    pub name: Option<String>,
    /// The nodes that label it.
    pub labelled_by: Vec<Id>,
    /// The value, as the component words it.
    pub value: Option<String>,
    /// The actions the node declares, in words.
    pub actions: Vec<&'static str>,
    /// The node is disabled.
    pub disabled: bool,
    /// The node takes focus or a click.
    pub interactive: bool,
}

/// One element an inspected region showed: a component, or a part of one with a node, a hit area or a focus stop.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Element {
    /// The component.
    pub id: Id,
    /// The part, `None` for the component itself.
    pub part: Option<PartRef>,
    /// The component's type name, without its path.
    pub component: &'static str,
    /// The number of components it sits in within the region.
    pub depth: usize,
    /// Its box, in the points of the region's layer.
    pub rect: Rect,
    /// The box inside its padding.
    pub content: Rect,
    /// What its node says; `None` for a component without a node.
    pub facts: Option<Facts>,
    /// Its place in Tab order within the region, from 1, when keyboard focus reaches it.
    pub stop: Option<usize>,
    /// The command bound to the control, when it has one.
    pub command: Option<CommandRef>,
}

impl Element {
    /// The id of the access node the element stands for: the component's own id, or the sub-node of its part. A
    /// member of a collection's run is recorded as such a part of the collection.
    pub fn node(&self) -> Id {
        self.part.map_or(self.id, |part| crate::runtime::access::sub_node_id(self.id, part))
    }
}

/// A hit area the sensing core registered for a component of the region.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HitArea {
    /// The component the area belongs to.
    pub owner: Id,
    /// The part, `None` for the component's own area.
    pub part: Option<PartRef>,
    /// The area at its least size across, in the points of the region's layer.
    pub rect: Rect,
}

/// What an inspected region showed in a pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Recorded {
    /// The region's layer, which the rects of its elements and hit areas are in.
    pub layer: areas::LayerKey,
    /// The elements in tree order: a component before what it shows, its parts right after it.
    pub elements: Vec<Element>,
    /// The hit areas, in the tree order of their components, a component's from the top start.
    pub hits: Vec<HitArea>,
}

/// A replay at another place keeps the record its run made, moved with it.
impl crate::runtime::Shift for Arc<Recorded> {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        let elements =
            self.elements.iter().map(|element| Element { rect: by.rect(element.rect), content: by.rect(element.content), ..element.clone() });
        let hits = self.hits.iter().map(|hit| HitArea { rect: by.rect(hit.rect), ..hit.clone() });
        Arc::new(Recorded { layer: self.layer, elements: elements.collect(), hits: hits.collect() })
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        if self.elements.len() != to.elements.len() || self.hits.len() != to.hits.len() {
            return Arc::clone(self);
        }
        let elements = self.elements.iter().zip(&to.elements).map(|(element, to)| Element {
            rect: element.rect.snapped(&to.rect, within),
            content: element.content.snapped(&to.content, within),
            ..element.clone()
        });
        let hits = self.hits.iter().zip(&to.hits).map(|(hit, to)| HitArea { rect: hit.rect.snapped(&to.rect, within), ..hit.clone() });
        Arc::new(Recorded { layer: self.layer, elements: elements.collect(), hits: hits.collect() })
    }
}

impl Recorded {
    /// The rect of the element at `index` where Tab stops on it: its hit area when it has one, else its box.
    pub fn stop_rect(&self, index: usize) -> Option<Rect> {
        let element = self.elements.get(index)?;
        let hit = self.hits.iter().find(|hit| hit.owner == element.id && hit.part == element.part);
        Some(hit.map_or(element.rect, |hit| hit.rect))
    }

    /// The stop rect of the element at `index` (as [`Recorded::stop_rect`]) mapped into the layer `target`.
    ///
    /// A region on another layer than its reader (a title bar's toolbar read from the ribbon's layer) has rects in
    /// its own layer's points; this maps them through the screen, so the reader never composes transforms itself.
    pub fn rect_in(&self, ctx: &Context, index: usize, target: areas::LayerKey) -> Option<Rect> {
        let rect = self.stop_rect(index)?;
        if target == self.layer {
            return Some(rect);
        }
        let to_target = areas::layer_to_screen(ctx, target).inverse() * areas::layer_to_screen(ctx, self.layer);
        Some(to_target * rect)
    }
}

/// A region being recorded.
struct Open {
    /// The layer the region is shown on, which the record's rects are in.
    layer: Layer,
    elements: Vec<(Element, Layer)>,
    parts: Vec<Element>,
    /// The elements whose show has not finished, outermost first.
    stack: Vec<usize>,
    /// The nodes a label scope named by its label, with the label's node.
    labels: Vec<(Id, Id)>,
    /// The text value noted for each component, kept for its facts, which may arrive later in the pass.
    text_values: IdMap<String>,
}

#[derive(Default)]
struct Kept {
    recorded: Option<Arc<Recorded>>,
    pointed: Option<usize>,
    pass: u64,
}

/// How many passes a region's record is kept after it was last shown.
const KEEP_PASSES: u64 = 4;

/// Shows `body` as the inspected region `key`: the components it shows in a real run are recorded, and the record is
/// returned and kept for [`recorded`]. A sizing or measuring run records nothing and returns `None`.
///
/// A record that differs from the kept one, after something earlier in the pass read it, reruns the pass, so a
/// reader shown before the region gets this pass's record.
pub fn record<R>(ui: &mut Ui, key: Id, body: impl FnOnce(&mut Ui) -> R) -> (R, Option<Arc<Recorded>>) {
    let ctx = ui.ctx().clone();
    if ui.is_sizing_pass() || measure::active(&ctx) {
        return (body(ui), None);
    }
    let open =
        Open { layer: ui.layer_id(), elements: Vec::new(), parts: Vec::new(), stack: Vec::new(), labels: Vec::new(), text_values: IdMap::default() };
    let noted = crate::runtime::replay::mark(&ctx);
    pass_state::with::<Log, _>(&ctx, |log| log.open.push(open));
    let out = body(ui);
    let Some(open) = pass_state::with::<Log, _>(&ctx, |log| log.open.pop()) else { return (out, None) };
    // What the body's components noted belongs to this region: a replay of a view around it adds only the region's
    // own record (below), not those elements to whatever region is open there.
    if let Some(noted) = noted {
        crate::runtime::replay::withdraw(&ctx, noted, replay_step);
    }
    let recorded = Arc::new(finish(&ctx, open));
    keep(&ctx, key, &recorded);
    // A memoized view that replays this region keeps its record as the run left it.
    crate::runtime::replay::journal(&ctx, || (key, Arc::clone(&recorded)), |ctx, (key, recorded)| keep(ctx, *key, recorded));
    (out, Some(recorded))
}

/// Keeps `recorded` as the record of the region `key`.
fn keep(ctx: &Context, key: Id, recorded: &Arc<Recorded>) {
    write(ctx, key, |kept| {
        let changed = kept.recorded.as_deref() != Some(&**recorded);
        kept.recorded = Some(Arc::clone(recorded));
        changed
    });
}

/// The latest record of the region `key`, read as a value: when the region shows later in the pass with a different
/// record, the pass runs again.
pub fn recorded(ctx: &Context, key: Id) -> Option<Arc<Recorded>> {
    note_read(ctx, key);
    pass_state::with::<Log, _>(ctx, |log| log.kept.get(&key).and_then(|kept| kept.recorded.clone()))
}

/// The element of the region `key` its list points at (the row under the pointer), read as a value.
pub fn pointed(ctx: &Context, key: Id) -> Option<usize> {
    note_read(ctx, pointer_key(key));
    pass_state::with::<Log, _>(ctx, |log| log.kept.get(&key).and_then(|kept| kept.pointed))
}

/// Points the list of the region `key` at the element at `index`, or at none. A region shown earlier in the pass that
/// read the old value is shown again.
pub fn point_at(ctx: &Context, key: Id, index: Option<usize>) {
    if measure::active(ctx) {
        return;
    }
    let changed = pass_state::with::<Log, _>(ctx, |log| {
        let kept = log.kept.entry(key).or_default();
        std::mem::replace(&mut kept.pointed, index) != index
    });
    if changed {
        edited(ctx, BindKey::Record(pointer_key(key)), key);
    }
}

fn pointer_key(key: Id) -> Id {
    key.with("pointed")
}

fn note_read(ctx: &Context, key: Id) {
    reads::note(ctx, BindKey::Record(key), Id::NULL, measure::active(ctx), &[]);
}

/// Keeps what `edit` writes under `key`, and reruns the pass when it changed a record read earlier in the pass.
fn write(ctx: &Context, key: Id, edit: impl FnOnce(&mut Kept) -> bool) {
    let pass = ctx.cumulative_pass_nr();
    let changed = pass_state::with::<Log, _>(ctx, |log| {
        log.kept.retain(|_, kept| kept.pass + KEEP_PASSES >= pass);
        let kept = log.kept.entry(key).or_default();
        kept.pass = pass;
        edit(kept)
    });
    if changed {
        edited(ctx, BindKey::Record(key), key);
    }
}

/// Notes an edit of the record `read` that `editor` made: a settled memoized view that read it runs, and so does the
/// pass when something read it earlier in the pass.
pub(crate) fn edited(ctx: &Context, read: BindKey, editor: Id) {
    crate::runtime::lazy::note_edit(ctx, read);
    if reads::earlier(ctx, read, editor).other {
        log::debug!(target: crate::runtime::egui::sizing::RERUN_LOG, "{editor:?} edited {read:?}, read earlier in the pass");
        request_discard(ctx, Rerun::Edit);
    }
}

/// Adds the region's hit areas and Tab order, maps every rect into the region's layer and puts each part after its
/// component.
fn finish(ctx: &Context, open: Open) -> Recorded {
    let from_screen = areas::layer_to_screen(ctx, open.layer).inverse();
    let to_region = |layer: Layer, rect: Rect| from_screen * (areas::layer_to_screen(ctx, layer) * rect);
    let ids: IdSet = open.elements.iter().map(|(element, _)| element.id).collect();
    let mut elements: Vec<Element> = Vec::with_capacity(open.elements.len() + open.parts.len());
    let mut parts = open.parts;
    let labels = open.labels;
    for (mut element, layer) in open.elements {
        element.rect = to_region(layer, element.rect);
        element.content = to_region(layer, element.content);
        let id = element.id;
        elements.push(element);
        for mut part in parts.extract_if(.., |part| part.id == id) {
            part.rect = to_region(layer, part.rect);
            part.content = part.rect;
            elements.push(part);
        }
    }
    let mut hits: Vec<HitArea> = pointer::hit_areas(ctx, |owner| ids.contains(&owner))
        .into_iter()
        .map(|(owner, part, rect)| HitArea { owner, part, rect: from_screen * rect })
        .collect();
    // Keep the first occurrence, matching the tree-order lookup when an id appears more than once.
    let mut index = std::collections::HashMap::new();
    for (at, element) in elements.iter().enumerate() {
        index.entry((element.id, element.part)).or_insert(at);
    }
    for (node, label) in labels {
        let facts = index.get(&(node, None)).and_then(|&at| elements[at].facts.as_mut());
        if let Some(facts) = facts {
            facts.labelled_by.push(label);
        }
    }
    name_from_labels(&mut elements);
    // The sensing core keeps its shapes by root, in no order; a record compares equal to the same region's next one.
    let place: IdMap<usize> = elements.iter().enumerate().rev().map(|(index, element)| (element.id, index)).collect();
    hits.sort_by(|a, b| {
        let key = |hit: &HitArea| (place.get(&hit.owner).copied(), hit.rect.min.y, hit.rect.min.x);
        key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
    });
    for (stop, (id, part)) in focus::tab_order(ctx, |id| ids.contains(&id)).into_iter().enumerate() {
        let at = match index.get(&(id, part)).copied() {
            Some(at) => at,
            None => {
                // A part with no node of its own: listed after its component, where its hit area is.
                let Some(owner) = index.get(&(id, None)).copied() else { continue };
                let rect = hits.iter().find(|hit| hit.owner == id && hit.part == part).map_or(elements[owner].rect, |hit| hit.rect);
                let depth = elements[owner].depth + 1;
                let element =
                    Element { id, part, component: elements[owner].component, depth, rect, content: rect, facts: None, stop: None, command: None };
                let at = owner + 1;
                elements.insert(at, element);
                for old in index.values_mut() {
                    if *old >= at {
                        *old += 1;
                    }
                }
                index.insert((id, part), at);
                at
            }
        };
        elements[at].stop = Some(stop + 1);
    }
    Recorded { layer: open.layer, elements, hits }
}

/// Names each node without a name of its own by the nodes of the region that label it, as a screen reader reads it.
fn name_from_labels(elements: &mut [Element]) {
    let names: IdMap<String> = elements
        .iter()
        .filter(|element| element.part.is_none())
        .filter_map(|element| Some((element.id, element.facts.as_ref()?.name.clone()?)))
        .collect();
    for facts in elements.iter_mut().filter_map(|element| element.facts.as_mut()).filter(|facts| facts.name.is_none()) {
        let by: Vec<&str> = facts.labelled_by.iter().filter_map(|id| names.get(id).map(String::as_str)).collect();
        if !by.is_empty() {
            facts.name = Some(by.join(" "));
        }
    }
}

/// A call a component made to the open region's record. Each is noted in the journal of the records open around it
/// (`replay::journal`), so a memoized view that replays inside a region adds to the region what its run added: the
/// elements, their parts, facts, commands and text values, and the labels that name them. The hit areas and the Tab order need no
/// step: the sensing and focus services replay their own registrations, and [`finish`] reads those for the elements.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    /// [`open_element`].
    Open { id: Id, component: &'static str, layer: Layer },
    /// [`close_element`].
    Close { id: Id, rect: Rect, content: Rect },
    /// [`note_command`].
    Command { id: Id, command: Option<CommandRef> },
    /// [`node_facts`].
    Facts { id: Id, part: Option<PartRef>, rect: Rect, facts: Facts },
    /// [`labelled`].
    Label { node: Id, label: Id },
    /// [`note_text_value`].
    TextValue { id: Id, value: String },
}

/// A replay at another place moves the boxes the step records.
impl crate::runtime::Shift for Step {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        match self {
            Step::Close { id, rect, content } => Step::Close { id: *id, rect: by.rect(*rect), content: by.rect(*content) },
            Step::Facts { id, part, rect, facts } => Step::Facts { id: *id, part: *part, rect: by.rect(*rect), facts: facts.clone() },
            other => other.clone(),
        }
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        match (self, to) {
            (Step::Close { id, rect, content }, Step::Close { rect: to_rect, content: to_content, .. }) => {
                Step::Close { id: *id, rect: rect.snapped(to_rect, within), content: content.snapped(to_content, within) }
            }
            (Step::Facts { id, part, rect, facts }, Step::Facts { rect: to_rect, .. }) => {
                Step::Facts { id: *id, part: *part, rect: rect.snapped(to_rect, within), facts: facts.clone() }
            }
            (other, _) => other.clone(),
        }
    }
}

/// Makes the call `step` stands for in the innermost open region; returns whether a region took it.
fn apply(ctx: &Context, step: &Step) -> bool {
    pass_state::with::<Log, _>(ctx, |log| {
        let Some(open) = log.open.last_mut() else { return false };
        match step {
            Step::Open { id, component, layer } => {
                let element = Element {
                    id: *id,
                    part: None,
                    component,
                    depth: open.stack.len(),
                    rect: Rect::NOTHING,
                    content: Rect::NOTHING,
                    facts: None,
                    stop: None,
                    command: None,
                };
                open.stack.push(open.elements.len());
                open.elements.push((element, *layer));
            }
            Step::Close { id, rect, content } => {
                let Some(at) = open.stack.iter().rposition(|index| open.elements[*index].0.id == *id) else { return true };
                let index = open.stack[at];
                open.stack.truncate(at);
                let element = &mut open.elements[index].0;
                element.rect = *rect;
                element.content = *content;
            }
            Step::Command { id, command } => {
                let Some(at) = open.stack.iter().rposition(|index| open.elements[*index].0.id == *id) else { return true };
                let index = open.stack[at];
                open.elements[index].0.command = command.clone();
            }
            Step::Facts { id, part, rect, facts } => {
                let Some(owner) = open.elements.iter_mut().rev().find(|(element, _)| element.id == *id) else { return true };
                match part {
                    None => {
                        let mut facts = facts.clone();
                        // A text value noted before the facts arrived; a value the node states itself wins.
                        if facts.value.is_none() {
                            facts.value = open.text_values.get(id).cloned();
                        }
                        owner.0.facts = Some(facts);
                    }
                    Some(part) => {
                        let (component, depth) = (owner.0.component, owner.0.depth + 1);
                        open.parts.retain(|element| !(element.id == *id && element.part == Some(*part)));
                        open.parts.push(Element {
                            id: *id,
                            part: Some(*part),
                            component,
                            depth,
                            rect: *rect,
                            content: *rect,
                            facts: Some(facts.clone()),
                            stop: None,
                            command: None,
                        });
                    }
                }
            }
            Step::Label { node, label } => open.labels.push((*node, *label)),
            Step::TextValue { id, value } => {
                // Kept for facts that arrive later in the pass, and written on facts that already did. A value the
                // node states itself wins over the text the adapter wrote.
                let value = open.text_values.entry(*id).or_insert_with(|| value.clone());
                let owner = open.elements.iter_mut().rev().find(|(element, _)| element.id == *id);
                if let Some(facts) = owner.and_then(|(element, _)| element.facts.as_mut()) {
                    facts.value.get_or_insert_with(|| value.clone());
                }
            }
        }
        true
    })
}

/// What a replay of a noted step calls.
fn replay_step(ctx: &Context, step: &Step) {
    apply(ctx, step);
}

/// Makes the call `step` stands for and, when a region took it, notes it for the replay of a memoized view around it.
/// With no region open nothing is built or noted.
fn run(ctx: &Context, step: Step) -> bool {
    let took = apply(ctx, &step);
    if took {
        crate::runtime::replay::journal(ctx, || step, replay_step);
    }
    took
}

/// Records that a label scope named the node of the component `node` by the label's node `label`.
pub(crate) fn labelled(ctx: &Context, node: Id, label: Id) {
    run(ctx, Step::Label { node, label });
}

/// Whether the components shown in `ui` report their nodes for an inspected region: a region is open and the run is
/// a real one.
pub(crate) fn records_nodes(ui: &Ui) -> bool {
    let ctx = ui.ctx();
    !ui.is_sizing_pass() && !measure::active(ctx) && pass_state::with::<Log, _>(ctx, |log| !log.open.is_empty())
}

/// Starts the record of the component `id` of type `component`, shown in `ui`, when an inspected region is open and
/// the run is a real one; returns whether it did.
pub(crate) fn open_element(ui: &Ui, id: Id, component: &'static str) -> bool {
    let ctx = ui.ctx();
    if measure::active(ctx) {
        return false;
    }
    // The type's own name: generic arguments go first, so `Select<'_, alloc::string::String>` is `Select`.
    let component = component.split('<').next().unwrap_or(component);
    let component = component.rsplit("::").next().unwrap_or(component);
    run(ctx, Step::Open { id, component, layer: ui.layer_id() })
}

/// Ends the record [`open_element`] started for `id`, with the box the component took and the box inside its padding.
pub(crate) fn close_element(ctx: &Context, id: Id, rect: Rect, content: Rect) {
    run(ctx, Step::Close { id, rect, content });
}

/// Notes the command bound to the control `id` in the open region's record; the report step calls it with the
/// component's resolved command. It does nothing outside a region or for a component whose record is not open.
pub(crate) fn note_command(ctx: &Context, id: Id, command: Option<CommandRef>) {
    run(ctx, Step::Command { id, command });
}

/// Notes the text value the text adapter wrote on the node of the component `id` (`report_text`), unless the node
/// states a value itself: the inspector then lists what AccessKit reads for a text field. The value is listed whether
/// the node's facts ([`node_facts`]) arrive before or after it in the pass.
pub(crate) fn note_text_value(ctx: &Context, id: Id, value: &str) {
    if recording(ctx) {
        run(ctx, Step::TextValue { id, value: value.to_owned() });
    }
}

/// Records what the node of the component `id`, or of its part `part` at `rect`, says.
pub(crate) fn node_facts(ctx: &Context, id: Id, part: Option<PartRef>, rect: Rect, facts: Facts) {
    run(ctx, Step::Facts { id, part, rect, facts });
}

/// The record's own order: a text value lists whether the node's facts arrive before or after it in the pass. No
/// component reports its facts after its text, so these trials drive the record's calls directly.
#[cfg(all(feature = "testing", any(test, feature = "suite")))]
pub(crate) mod trials {
    use super::{Facts, close_element, node_facts, note_text_value, open_element, record};
    use crate::testing::{Fixture, trial};
    use egui::Id;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The facts of a text input node that states `value` itself, or none.
    fn facts(value: Option<&str>) -> Facts {
        Facts {
            role: "TextInput".to_owned(),
            name: None,
            labelled_by: Vec::new(),
            value: value.map(str::to_owned),
            actions: Vec::new(),
            disabled: false,
            interactive: true,
        }
    }

    /// A text value noted before its node's facts arrive is listed once they do, as it is when they arrived first;
    /// facts reported again keep it, and a value the node states itself still wins.
    #[trial]
    fn a_text_value_is_listed_whichever_order_the_facts_arrive_in() {
        let key = Id::NULL.with("trial.inspect.text_value_order");
        let (before, after, again, own) = (key.with("before"), key.with("after"), key.with("again"), key.with("own"));
        let seen = Rc::new(RefCell::new(None));
        let out = Rc::clone(&seen);
        let mut fixture = Fixture::new(move |ui| {
            let ctx = ui.ctx().clone();
            let (_, recorded) = record(ui, key, |ui| {
                let rect = ui.max_rect();
                // The text value first, then the facts.
                open_element(ui, before, "Field");
                note_text_value(&ctx, before, "before");
                node_facts(&ctx, before, None, rect, facts(None));
                close_element(&ctx, before, rect, rect);
                // The order the report step takes: the facts, then the text value.
                open_element(ui, after, "Field");
                node_facts(&ctx, after, None, rect, facts(None));
                note_text_value(&ctx, after, "after");
                close_element(&ctx, after, rect, rect);
                // The facts, the text value, then the facts reported again.
                open_element(ui, again, "Field");
                node_facts(&ctx, again, None, rect, facts(None));
                note_text_value(&ctx, again, "again");
                node_facts(&ctx, again, None, rect, facts(None));
                close_element(&ctx, again, rect, rect);
                // The node states a value of its own, after the text value.
                open_element(ui, own, "Field");
                note_text_value(&ctx, own, "typed");
                node_facts(&ctx, own, None, rect, facts(Some("own")));
                close_element(&ctx, own, rect, rect);
            });
            if recorded.is_some() {
                *out.borrow_mut() = recorded;
            }
        });
        fixture.frame(Vec::new());
        let seen = seen.borrow();
        let Some(recorded) = seen.as_deref() else { panic!("the region ran") };
        let value = |id: Id| {
            let element = recorded.elements.iter().find(|element| element.id == id && element.part.is_none());
            element.and_then(|element| element.facts.as_ref()).and_then(|facts| facts.value.clone())
        };
        assert_eq!(value(before).as_deref(), Some("before"), "facts arriving after the text value list it: {:?}", recorded.elements);
        assert_eq!(value(after).as_deref(), Some("after"), "facts arriving before the text value list it");
        assert_eq!(value(again).as_deref(), Some("again"), "facts reported again keep the text value");
        assert_eq!(value(own).as_deref(), Some("own"), "a value the node states itself wins over the text value");
    }
}
