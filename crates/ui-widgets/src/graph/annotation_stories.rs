//! Stories of the graph's annotation layers: backdrops under the nodes, notes and markers over them.
//!
//! - Stories: `NodeGraph<'static, ()>`, beside the graph's own stories, as the list's view stories sit beside its
//!   stories.

use super::annotation::{AnnotationEvent, AnnotationKey, BACKDROP_GRIP, Backdrop, MARKER, Marker, NOTE, Note};
use super::{Node, NodeGraph, NodeId, NodeKind, PortRef, Wire};
use crate::contract::{Bind, Props as _, ShowCollection as _};
use crate::style::DataColor;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::keyed;
use egui::{Color32, Pos2, Rect, Ui, Vec2};

/// A node with an input on the left and an output on the right.
struct Step;

impl NodeKind<()> for Step {
    fn title(&self) -> &str {
        "Step"
    }

    fn port_count(&self) -> usize {
        2
    }

    fn port_name(&self, port: usize) -> String {
        let name = if port == 0 { "Input" } else { "Output" };
        name.to_owned()
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        Vec2::new(if port == 0 { 0.0 } else { size.x }, size.y / 2.0)
    }
}

const STEP: Step = Step;

/// The two nodes' rects, side by side, in document units.
const NODES: [Rect; 2] =
    [Rect { min: Pos2::new(48.0, 64.0), max: Pos2::new(188.0, 136.0) }, Rect { min: Pos2::new(228.0, 64.0), max: Pos2::new(368.0, 136.0) }];

/// A backdrop around both nodes, with room for its description below them.
const FRAME: Rect = Rect { min: Pos2::new(32.0, 40.0), max: Pos2::new(384.0, 200.0) };

/// A soft green tint, document data the app would store with the backdrop.
const TINT: DataColor = DataColor::new(Color32::from_rgba_premultiplied(40, 80, 48, 96));

const DESCRIPTION: &str = "Blends the two sources, then sharpens the result before it reaches the output.";

const LONG_NOTE: &str = "This note runs on past its rect's width, so it wraps onto further lines and its rect grows down to hold \
                         them; the description of the backdrop beside it is clipped to the backdrop instead.";

/// What one annotated graph shows: the backdrop's rect, the notes, the markers and the selection to start from.
struct Annotated<'a> {
    frame: Rect,
    notes: &'a [Note<'a>],
    markers: &'a [Marker<'a>],
    selected: Option<AnnotationKey>,
}

/// The two nodes, joined, with the annotations of `shown`; the selection lives in the story's data, and a resize
/// the backdrop reports moves the rect the story keeps, as an app would.
fn annotated_graph(ui: &mut Ui, data: &mut StoryData, shown: &Annotated<'_>) -> StoryOutput {
    let kinds: [&dyn NodeKind<()>; 2] = [&STEP, &STEP];
    let ids = [NodeId(0), NodeId(1)];
    let wires = [Wire::new(0, PortRef::new(NodeId(0), 1), PortRef::new(NodeId(1), 0))];
    let source = keyed(&ids, |id| *id);
    let frame = *data.value("frame", shown.frame);
    let backdrops = [Backdrop::new(0, frame, TINT).title("Blend and sharpen").description(DESCRIPTION)];
    let mut selected = *data.value("selected", shown.selected);
    let response = NodeGraph::new(&NODES, &kinds, &wires)
        .label("Graph")
        .backdrops(&backdrops)
        .notes(shown.notes)
        .markers(shown.markers)
        .annotation_selection(Bind::bound(&mut selected))
        .show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
    *data.value("selected", shown.selected) = selected;
    for event in &response.annotations {
        if let AnnotationEvent::Resize { edit, .. } = event {
            let _ = edit.apply(data.value("frame", shown.frame));
        }
    }
    response.into()
}

/// Presses the first backdrop's grip and moves it down and to the right, then lets go.
fn drag_grip(fixture: &mut Fixture) {
    if let Some(root) = fixture.root_id()
        && let Some(grip) = fixture.part_rect(root, BACKDROP_GRIP.at(0))
    {
        fixture.drag_at(grip.center(), Vec2::new(48.0, 32.0));
    }
}

const NOTES: [Note<'static>; 1] = [Note::new(1, Rect { min: Pos2::new(228.0, 150.0), max: Pos2::new(368.0, 190.0) }, "Check the radius")];
const MARKERS: [Marker<'static>; 1] = [Marker::new(2, Pos2::new(200.0, 30.0), "Review")];

/// A backdrop around two nodes, a note and a marker, nothing selected.
#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Never)]
fn annotated(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    annotated_graph(ui, data, &Annotated { frame: FRAME, notes: &NOTES, markers: &MARKERS, selected: None })
}

/// Clicks the note, then the marker: each takes the selection from the one before.
fn select_each(fixture: &mut Fixture) {
    let Some(root) = fixture.root_id() else { return };
    for part in [NOTE.at(1), MARKER.at(2)] {
        if let Some(rect) = fixture.part_rect(root, part) {
            fixture.click_at(rect.center());
        }
    }
}

/// The backdrop selected; the drive selects the note and then the marker in turn.
#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(select_each))]
fn annotation_selected(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    annotated_graph(ui, data, &Annotated { frame: FRAME, notes: &NOTES, markers: &MARKERS, selected: Some(AnnotationKey::Backdrop(0)) })
}

/// The backdrop's grip pressed and moved: the backdrop takes the size the drag reports.
#[story(NodeGraph<'static, ()>, covers = [Many], drive = Drive::Custom(drag_grip))]
fn backdrop_resizing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    annotated_graph(ui, data, &Annotated { frame: FRAME, notes: &[], markers: &[], selected: Some(AnnotationKey::Backdrop(0)) })
}

/// A note longer than its rect, which wraps and grows down, beside a backdrop whose description is clipped to it.
#[story(NodeGraph<'static, ()>, covers = [LongText], drive = Drive::Never)]
fn long_note(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let notes = [Note::new(1, Rect::from_min_size(Pos2::new(400.0, 64.0), Vec2::new(160.0, 40.0)), LONG_NOTE)];
    let small = Rect::from_min_max(FRAME.min, Pos2::new(FRAME.max.x, 150.0));
    annotated_graph(ui, data, &Annotated { frame: small, notes: &notes, markers: &MARKERS, selected: None })
}
