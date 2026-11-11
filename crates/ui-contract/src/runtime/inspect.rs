//! Decision records and findings: what each pipeline step did, kept in frames a test fixture drives and inside an
//! inspected region ([`record`]). The region's record is what the debug inspector draws: every component shown in it
//! with its box and access facts, the hit areas the sensing core registered and the focus stops in Tab order.

use crate::runtime::egui::areas;
use crate::runtime::egui::sizing::{Rerun, request_discard};
use crate::runtime::reads::{self, BindKey};
use crate::runtime::{focus, measure, pass_state, pointer};
use craft_behavior::PartRef;
use egui::{Context, Id, LayerId, Rect, Ui};
use std::collections::{HashMap, HashSet};
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
    steps: HashMap<Id, Vec<&'static str>>,
    findings: Vec<Finding>,
    canvas_controls: Vec<&'static str>,
    /// The inspected regions open in the running pass, innermost last.
    open: Vec<Open>,
    /// The latest record of each region, and the element its list points at, by the region's key.
    kept: HashMap<Id, Kept>,
}

/// Whether records are kept: in frames a test fixture drives, and inside an inspected region in any build.
pub(crate) fn recording(ctx: &Context) -> bool {
    fixture_frame() || pass_state::with::<Log, _>(ctx, |log| !log.open.is_empty())
}

#[cfg(feature = "testing")]
use crate::runtime::egui::fixture::driving as fixture_frame;

#[cfg(not(feature = "testing"))]
fn fixture_frame() -> bool {
    false
}

/// Starts the step record of `id` for a new show.
pub(crate) fn begin(ctx: &Context, id: Id) {
    if recording(ctx) {
        pass_state::with::<Log, _>(ctx, |log| log.steps.insert(id, Vec::new()));
    }
}

/// Records that `step` ran for `id`.
pub(crate) fn step(ctx: &Context, id: Id, step: &'static str) {
    if recording(ctx) {
        pass_state::with::<Log, _>(ctx, |log| log.steps.entry(id).or_default().push(step));
    }
}

/// The steps the latest show of `id` ran, in order.
#[cfg_attr(not(feature = "testing"), expect(dead_code, reason = "read by trials and the debug inspector"))]
pub(crate) fn steps_of(ctx: &Context, id: Id) -> Vec<&'static str> {
    pass_state::with::<Log, _>(ctx, |log| log.steps.get(&id).cloned().unwrap_or_default())
}

/// Reports a finding.
pub(crate) fn finding(ctx: &Context, finding: Finding) {
    if !recording(ctx) {
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
    if !recording(ctx) {
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
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Recorded {
    /// The elements in tree order: a component before what it shows, its parts right after it.
    pub elements: Vec<Element>,
    /// The hit areas, in the tree order of their components, a component's from the top start.
    pub hits: Vec<HitArea>,
}

impl Recorded {
    /// The rect of the element at `index` where Tab stops on it: its hit area when it has one, else its box.
    pub fn stop_rect(&self, index: usize) -> Option<Rect> {
        let element = self.elements.get(index)?;
        let hit = self.hits.iter().find(|hit| hit.owner == element.id && hit.part == element.part);
        Some(hit.map_or(element.rect, |hit| hit.rect))
    }
}

/// A region being recorded.
struct Open {
    /// The layer the region is shown on, which the record's rects are in.
    layer: LayerId,
    elements: Vec<(Element, LayerId)>,
    parts: Vec<Element>,
    /// The elements whose show has not finished, outermost first.
    stack: Vec<usize>,
    /// The nodes a label scope named by its label, with the label's node.
    labels: Vec<(Id, Id)>,
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
    let open = Open { layer: ui.layer_id(), elements: Vec::new(), parts: Vec::new(), stack: Vec::new(), labels: Vec::new() };
    pass_state::with::<Log, _>(&ctx, |log| log.open.push(open));
    let out = body(ui);
    let Some(open) = pass_state::with::<Log, _>(&ctx, |log| log.open.pop()) else { return (out, None) };
    let recorded = Arc::new(finish(&ctx, open));
    write(&ctx, key, |kept| {
        let changed = kept.recorded.as_deref() != Some(&*recorded);
        kept.recorded = Some(Arc::clone(&recorded));
        changed
    });
    (out, Some(recorded))
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
    if changed && reads::earlier(ctx, BindKey::Record(pointer_key(key)), key).other {
        request_discard(ctx, Rerun::Edit);
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
    if changed && reads::earlier(ctx, BindKey::Record(key), key).other {
        request_discard(ctx, Rerun::Edit);
    }
}

/// Adds the region's hit areas and Tab order, maps every rect into the region's layer and puts each part after its
/// component.
fn finish(ctx: &Context, open: Open) -> Recorded {
    let from_screen = areas::layer_to_screen(ctx, open.layer).inverse();
    let to_region = |layer: LayerId, rect: Rect| from_screen * (areas::layer_to_screen(ctx, layer) * rect);
    let ids: HashSet<Id> = open.elements.iter().map(|(element, _)| element.id).collect();
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
    for (node, label) in labels {
        let facts = elements.iter_mut().find(|element| element.id == node && element.part.is_none()).and_then(|element| element.facts.as_mut());
        if let Some(facts) = facts {
            facts.labelled_by.push(label);
        }
    }
    name_from_labels(&mut elements);
    // The sensing core keeps its shapes by root, in no order; a record compares equal to the same region's next one.
    let place: HashMap<Id, usize> = elements.iter().enumerate().rev().map(|(index, element)| (element.id, index)).collect();
    hits.sort_by(|a, b| {
        let key = |hit: &HitArea| (place.get(&hit.owner).copied(), hit.rect.min.y, hit.rect.min.x);
        key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
    });
    for (at, (id, part)) in focus::tab_order(ctx, |id| ids.contains(&id)).into_iter().enumerate() {
        let index = match elements.iter().position(|element| element.id == id && element.part == part) {
            Some(index) => index,
            None => {
                // A part with no node of its own: listed after its component, where its hit area is.
                let Some(owner) = elements.iter().position(|element| element.id == id && element.part.is_none()) else { continue };
                let rect = hits.iter().find(|hit| hit.owner == id && hit.part == part).map_or(elements[owner].rect, |hit| hit.rect);
                let depth = elements[owner].depth + 1;
                let element = Element { id, part, component: elements[owner].component, depth, rect, content: rect, facts: None, stop: None };
                elements.insert(owner + 1, element);
                owner + 1
            }
        };
        elements[index].stop = Some(at + 1);
    }
    Recorded { elements, hits }
}

/// Names each node without a name of its own by the nodes of the region that label it, as a screen reader reads it.
fn name_from_labels(elements: &mut [Element]) {
    let names: HashMap<Id, String> = elements
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

/// Records that a label scope named the node of the component `node` by the label's node `label`.
pub(crate) fn labelled(ctx: &Context, node: Id, label: Id) {
    pass_state::with::<Log, _>(ctx, |log| {
        if let Some(open) = log.open.last_mut() {
            open.labels.push((node, label));
        }
    });
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
    let layer = ui.layer_id();
    pass_state::with::<Log, _>(ctx, |log| {
        let Some(open) = log.open.last_mut() else { return false };
        let component = component.rsplit("::").next().unwrap_or(component);
        let component = component.split('<').next().unwrap_or(component);
        let element =
            Element { id, part: None, component, depth: open.stack.len(), rect: Rect::NOTHING, content: Rect::NOTHING, facts: None, stop: None };
        open.stack.push(open.elements.len());
        open.elements.push((element, layer));
        true
    })
}

/// Ends the record [`open_element`] started for `id`, with the box the component took and the box inside its padding.
pub(crate) fn close_element(ctx: &Context, id: Id, rect: Rect, content: Rect) {
    pass_state::with::<Log, _>(ctx, |log| {
        let Some(open) = log.open.last_mut() else { return };
        let Some(at) = open.stack.iter().rposition(|index| open.elements[*index].0.id == id) else { return };
        let index = open.stack[at];
        open.stack.truncate(at);
        let element = &mut open.elements[index].0;
        element.rect = rect;
        element.content = content;
    });
}

/// Records what the node of the component `id`, or of its part `part` at `rect`, says.
pub(crate) fn node_facts(ctx: &Context, id: Id, part: Option<PartRef>, rect: Rect, facts: Facts) {
    pass_state::with::<Log, _>(ctx, |log| {
        let Some(open) = log.open.last_mut() else { return };
        let Some(owner) = open.elements.iter_mut().rev().find(|(element, _)| element.id == id) else { return };
        match part {
            None => owner.0.facts = Some(facts),
            Some(part) => {
                let (component, depth) = (owner.0.component, owner.0.depth + 1);
                open.parts.retain(|element| !(element.id == id && element.part == Some(part)));
                open.parts.push(Element { id, part: Some(part), component, depth, rect, content: rect, facts: Some(facts), stop: None });
            }
        }
    });
}
