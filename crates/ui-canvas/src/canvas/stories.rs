//! Stories of the canvas family.

use super::{
    BACKGROUND, Canvas, CanvasSpace, CanvasTool, ContextSource, CropOverlay, MiddleButton, NavigationOptions, NoTool, RasterDisplay, ToolAction,
    ToolContext, ToolHost, ToolPath, ToolPointer, ViewTarget, WheelScheme, canvas_space, tool_pass, with_canvas_space,
};
use crate::buttons::Button;
use crate::canvas::testing::canvas_story::{StoryDocument, StoryPicture, StoryShape, accent, paint_shapes, show_document};
use crate::canvas_controls::CanvasGrid;
use crate::contract::{
    Bind, BodyCx, CanvasGesture, Gap, Props as _, Sampler as _, Show as _, ShowCollection as _, ShowContainer as _, clicked, tooltip,
};
use crate::dialog::Dialog;
use crate::display::Text;
use crate::graph::{Node, NodeGraph, NodeId, NodeKind, PortDirection, PortRef, Wire};
use crate::layout_components::Stack;
use crate::primitives::{Draw, EditorPurpose, EditorState};
use crate::runtime::access::Role;
use crate::runtime::env::Env;
use crate::testing::{Drive, Fixture, Grip, StoryData, StoryOutput, pointer, stage, story};
use craft_behavior::geometry::CropBox;
use craft_behavior::kurbo::{Affine, Point, Rect as DocRect, Shape as _};
use craft_behavior::{CancelReason, CanvasView, EditorInput, TextPos, Wheel, keyed};
use egui::{Color32, Key, Modifiers, Pos2, Rect, Ui, Vec2, pos2, vec2};

/// The zoomed story's zoom, in percent of one device pixel per document pixel.
const ZOOMED: f64 = 200.0;

fn crop() -> CropBox {
    CropBox::upright(DocRect::new(240.0, 145.0, 400.0, 255.0))
}

#[story(Canvas, covers = [Default])]
fn crop_tool(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        CropOverlay::new("Crop", Bind::given(crop())).show_at(ui, body.rect)
    });
    shown.output.into()
}

#[story(Canvas, covers = [Default])]
fn zoomed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), ZOOMED);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        CropOverlay::new("Crop", Bind::given(crop())).show_at(ui, body.rect)
    });
    shown.output.into()
}

/// The node graph story's nodes, in document units: a row near the origin and one far off at negative points, so the
/// view opens fitted to all of them and a pan finds the far one.
const GRAPH_NODES: [Rect; 4] = [
    Rect::from_min_max(pos2(0.0, 0.0), pos2(140.0, 72.0)),
    Rect::from_min_max(pos2(200.0, 60.0), pos2(340.0, 132.0)),
    Rect::from_min_max(pos2(400.0, 0.0), pos2(540.0, 72.0)),
    Rect::from_min_max(pos2(-600.0, -400.0), pos2(-460.0, -328.0)),
];

/// The node graph story's grid step, in document units.
const GRID_STEP: f32 = 40.0;

/// The story's node: an input on the left, an output on the right.
struct StoryNode;

impl NodeKind for StoryNode {
    fn title(&self) -> &str {
        "Node"
    }

    fn port_count(&self) -> usize {
        2
    }

    fn port_name(&self, port: usize) -> String {
        if port == 0 { "Input" } else { "Output" }.to_owned()
    }

    fn port_direction(&self, port: usize) -> PortDirection {
        if port == 0 { PortDirection::Input } else { PortDirection::Output }
    }

    fn port_offset(&self, port: usize, size: Vec2) -> Vec2 {
        Vec2::new(if port == 0 { 0.0 } else { size.x }, size.y / 2.0)
    }
}

/// The rect of every node of the story, which Fit on Screen frames.
fn graph_extent() -> DocRect {
    GRAPH_NODES.iter().fold(DocRect::new(0.0, 0.0, 0.0, 0.0), |all, node| {
        all.union(DocRect::new(f64::from(node.min.x), f64::from(node.min.y), f64::from(node.max.x), f64::from(node.max.y)))
    })
}

/// A node graph on a canvas of a document with no edges: the view opens fitted to the nodes, the wheel zooms about
/// the pointer and a middle drag pans past any edge, and the nodes follow the view through its layer transform over
/// the canvas grid.
#[story(Canvas, covers = [Default])]
fn node_graph_on_a_canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let (view, view_at) = data.value_at("view", CanvasView::new(Rect::NOTHING, Point::ZERO, 1.0));
    let kinds: Vec<&dyn NodeKind> = vec![&StoryNode; GRAPH_NODES.len()];
    let ids: Vec<NodeId> = (0..GRAPH_NODES.len() as u32).map(NodeId).collect();
    let wires = [Wire::new(0, PortRef::new(NodeId(0), 1), PortRef::new(NodeId(1), 0)), Wire::new(1, PortRef::new(NodeId(1), 1), PortRef::new(NodeId(2), 0))];
    let source = keyed(&ids, |id| *id);
    Canvas::new("Graph")
        .document(ViewTarget::unbounded(graph_extent()))
        .view(view_at.bind(&mut *view))
        .show_at(ui, room, |ui, body| {
            let Some(space) = canvas_space(&body.env) else { return };
            let mut graph = NodeGraph::new(&GRAPH_NODES, &kinds, &wires).label("Nodes");
            if let Some(zoom) = space.view.layer_transform() {
                graph = graph.zoom(zoom);
            }
            let _ = graph.show_items(ui, &source, &mut |id, _| Node::new(*id, kinds[id.0 as usize]));
            // In the picture slot, under the nodes.
            let _ = CanvasGrid::new("Grid").spacing((GRID_STEP, GRID_STEP).into()).picture().show_at(ui, body.rect);
        })
        .output
        .into()
}

/// The side of the surface story's marker picture, in pixels.
const MARKER_SIDE: u32 = 160;

/// The surface story's tool, as a 3D view's orbit: a drag turns the marker by its sideways travel and the wheel
/// scales it. The canvas has no view, so the wheel and the press are the tool's.
struct TurnTool {
    /// The marker's turn, in degrees.
    turn: f64,
    /// The marker's scale.
    scale: f64,
    /// The pointer at the last press or drag of a running gesture.
    last: Option<Point>,
    /// The turn when the gesture began, which a cancel restores.
    pressed_turn: f64,
}

/// Degrees the marker turns per point of sideways travel.
const TURN_PER_POINT: f64 = 0.5;

impl CanvasTool for TurnTool {
    fn press(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.last = Some(Point::new(f64::from(pointer.pos.x), f64::from(pointer.pos.y)));
        self.pressed_turn = self.turn;
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) {
        let at = Point::new(f64::from(pointer.pos.x), f64::from(pointer.pos.y));
        if let Some(last) = self.last {
            self.turn += (at.x - last.x) * TURN_PER_POINT;
        }
        self.last = Some(at);
    }

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {
        self.last = None;
    }

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {
        self.turn = self.pressed_turn;
        self.last = None;
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn wheel(&mut self, wheel: &Wheel, _: Pos2, _: &mut ToolContext<'_>) -> bool {
        // A wheel line is a notch; a device that scrolls in points takes one every 50 of them.
        let notches = if wheel.lines.y != 0.0 { wheel.lines.y } else { wheel.scroll.y / 50.0 };
        let factor = 1.1_f64.powf(f64::from(notches)) * f64::from(wheel.zoom);
        if !(factor.is_finite() && factor > 0.0) || factor == 1.0 {
            return false;
        }
        self.scale = (self.scale * factor).clamp(0.25, 4.0);
        true
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the marker turns by dragging only")
    }
}

/// Shows the marker turned and scaled at the middle of `at`: a square with a bar from its centre to one edge, so the
/// turn shows.
fn show_marker(ui: &mut Ui, at: Rect, tool: &TurnTool) {
    let middle = f64::from(MARKER_SIDE) / 2.0;
    let place = Affine::translate((middle, middle)) * Affine::rotate(tool.turn.to_radians()) * Affine::scale(tool.scale);
    let ink = accent(ui, 1.0);
    let shapes = [
        StoryShape::filled(&(place * DocRect::new(-30.0, -30.0, 30.0, 30.0).to_path(0.1)), ink.gamma_multiply(0.4)).outlined(2.0, ink),
        StoryShape::filled(&(place * DocRect::new(0.0, -4.0, 30.0, 4.0).to_path(0.1)), ink),
    ];
    // A new revision for every turn and scale, so the display reads the picture again.
    let revision = (tool.turn.to_bits() ^ tool.scale.to_bits().rotate_left(17)).max(1);
    let picture = StoryPicture::new(MARKER_SIDE, MARKER_SIDE, |p| paint_shapes(&shapes, Color32::TRANSPARENT, p)).revision(revision);
    // The marker is a picture of its own, shown 1:1 in a space of its own: the canvas has no view to show it through.
    let view = CanvasView::new(at, Point::new(middle, middle), 1.0);
    let space = CanvasSpace::new(view, ui.pixels_per_point());
    with_canvas_space(ui, space, |ui| {
        let _ = RasterDisplay::new("Marker", &picture).show_at(ui, at);
    });
}

/// A canvas without a view, as a 3D view uses one: no pan and no zoom of its own, so a drag reaches the tool, which
/// turns the marker, and the wheel reaches the tool, which scales it ([`CanvasTool::wheel`]).
#[story(Canvas, covers = [Default], drive = Drive::Drag(Grip::new(BACKGROUND)))]
fn a_surface_without_a_view(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let mut tool = TurnTool {
        turn: *data.value("turn", 0.0),
        scale: *data.value("scale", 1.0),
        last: *data.value::<Option<Point>>("last", None),
        pressed_turn: *data.value("pressed turn", 0.0),
    };
    let shown = Canvas::new("Surface").show_at(ui, room, |ui, body| {
        if let Some(pass) = tool_pass(&body.env) {
            let _ = ToolHost::new().run(ui, &pass, &mut tool);
        }
        show_marker(ui, body.rect, &tool);
    });
    *data.value("turn", 0.0) = tool.turn;
    *data.value("scale", 1.0) = tool.scale;
    *data.value::<Option<Point>>("last", None) = tool.last;
    *data.value("pressed turn", 0.0) = tool.pressed_turn;
    shown.output.into()
}

#[story(Canvas, covers = [Default])]
fn freehand(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    Canvas::new("Painting")
        .view(StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0))
        .show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
        .output
        .into()
}

/// The canvas under an open modal dialog that keeps it live: the wheel scrolls and zooms it, the middle button and
/// Space pan it, and a click on the image picks a point into the dialog. Every other input stays the dialog's.
#[story(Canvas, covers = [], drive = Drive::Never)]
fn under_a_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let pixels_per_point = ui.pixels_per_point();
    let mut open = *data.value("open", false);
    let mut picked = *data.value::<Option<Point>>("picked", None);
    let row = |ui: &mut Ui, _: &mut BodyCx<'_>| {
        if clicked(&Button::new("Pick a Point…").show(ui)) {
            open = true;
        }
    };
    let stage = Stack::new().fill().footer(row).footer_gap(Gap::Stack).show_at(ui, room, |ui, stack| {
        let (view, view_at) = data.value_at("view", StoryDocument::STANDARD.view(stack.rect, pixels_per_point, 100.0));
        Canvas::new("Document")
            .view(view_at.bind(view))
            .show_at(ui, stack.rect, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
            .output
    });
    let Some(canvas) = stage.inner else { return StoryOutput::default() };
    // The press the dialog lets through reaches the canvas's background, where the app's tool would take it.
    if open && let (Some(at), Some(space)) = (canvas.click_at(), canvas.space) {
        picked = Some(space.view.to_doc(at));
    }
    let id = canvas.response.id;
    Dialog::new("Pick a Point").opened(Bind::bound(&mut open)).show(ui, |ui, cx| {
        cx.keep_view_open(id);
        cx.pass_canvas_gesture(id, CanvasGesture::PRIMARY);
        let text = picked.map_or_else(|| "Click the image to pick a point.".to_owned(), |at| format!("Picked {:.0}, {:.0}", at.x, at.y));
        Text::new(text).show(ui);
    });
    *data.value("open", false) = open;
    *data.value::<Option<Point>>("picked", None) = picked;
    canvas.into()
}

/// The centre of the story's canvas.
fn canvas_centre(fixture: &Fixture) -> Option<egui::Pos2> {
    fixture.by_role(Role::CANVAS).first().map(|canvas| canvas.rect.center())
}

/// Middle-clicks the canvas's centre: the autoscroll mark stands where the click came up.
fn middle_click_centre(fixture: &mut Fixture) {
    let Some(at) = canvas_centre(fixture) else { return };
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::middle(at, true)]);
    fixture.frame(vec![pointer::middle(at, false)]);
}

/// Clicks the canvas, so it has focus, and presses Shift+F10.
fn press_menu_key(fixture: &mut Fixture) {
    let Some(at) = canvas_centre(fixture) else { return };
    fixture.click_at(at);
    fixture.key(Key::F10, Modifiers::SHIFT).settle();
}

/// A document editor's wheel: the wheel scrolls, Shift + wheel scrolls sideways, and Ctrl or Cmd + wheel steps the zoom
/// ladder.
#[story(Canvas, covers = [Default])]
fn document_wheel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let navigation = NavigationOptions { wheel: WheelScheme::Document, ..NavigationOptions::default() };
    let (view, view_at) = data.value_at("view", scrolled_view(room, ui.pixels_per_point()));
    Canvas::new("Document")
        .navigation(navigation)
        .view(view_at.bind(&mut *view))
        .show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD))
        .output
        .into()
}

/// The standard document zoomed in and scrolled off its origin, so a story that scrolls opens away from the start.
fn scrolled_view(room: Rect, pixels_per_point: f32) -> craft_behavior::CanvasView {
    let mut view = StoryDocument::STANDARD.view(room, pixels_per_point, ZOOMED);
    view.center = Point::new(view.center.x + 60.0, view.center.y + 40.0);
    view
}

/// The middle button's autoscroll, started by a middle click: the mark stands where the click came up, and the view
/// scrolls toward the pointer once it leaves the mark's dead zone.
#[story(Canvas, covers = [], drive = Drive::Custom(middle_click_centre))]
fn middle_autoscroll(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let navigation = NavigationOptions { middle_button: MiddleButton::AutoScroll, ..NavigationOptions::default() };
    let (view, view_at) = data.value_at("view", scrolled_view(room, ui.pixels_per_point()));
    let shown = Canvas::new("Document")
        .navigation(navigation)
        .view(view_at.bind(&mut *view))
        .show_at(ui, room, |ui, body| show_document(ui, body.rect, &StoryDocument::STANDARD));
    shown.output.into()
}

/// A canvas with an editor part: a press on it gives the part key focus and the input method, the body reads the typed
/// text from its pass and reports its caret back through the pass's editor.
#[story(Canvas, covers = [Default])]
fn editor_part(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0);
    let typed = data.value("typed", String::from("Typed text"));
    let shown = Canvas::new("Page").editor(EditorPurpose::Text).view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        let Some(pass) = tool_pass(&body.env) else { return };
        for input in pass.editor_input.iter() {
            if let EditorInput::Text(text) = input {
                typed.push_str(text);
            }
        }
        // A caret one text line tall at the canvas's top left, past what was typed, as a document editor reports its
        // own caret.
        let line = Env::of(ui).tokens.typography.body.line_height;
        let at = Rect::from_min_size(body.rect.min, vec2(body.rect.width(), line));
        let caret = Rect::from_min_size(at.min, vec2(0.0, line));
        if let Some(editor) = &pass.editor {
            editor.report(ui.ctx(), EditorState::new(caret, &[], TextPos::new(0, typed.len())));
        }
        if !typed.is_empty() {
            Text::new(typed.clone()).show_at(ui, at);
        }
    });
    shown.output.into()
}

/// A tool that asks for a pointer tip on every hover and takes presses, as a document editor's text tool tips a link.
struct TippingTool;

impl CanvasTool for TippingTool {
    fn hover(&mut self, _: &ToolPointer, cx: &mut ToolContext<'_>) {
        cx.tip("Ctrl+click to follow the link");
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Follow Link command")
    }
}

/// Rests the pointer on the canvas's centre past the tooltip delay, so the tool's tip shows.
fn rest_on_centre(fixture: &mut Fixture) {
    let Some(at) = canvas_centre(fixture) else { return };
    fixture.frame(vec![pointer::moved(at)]);
    fixture.advance(tooltip::DELAY);
    fixture.settle();
}

/// A tool's pointer tip ([`ToolContext::tip`]): it shows at the pointer after the tooltip delay while the pointer rests
/// on the canvas, and a press hides it.
#[story(Canvas, covers = [], drive = Drive::Custom(rest_on_centre))]
fn tool_tip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let view = StoryDocument::STANDARD.view(room, ui.pixels_per_point(), 100.0);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        if let Some(pass) = tool_pass(&body.env) {
            let _ = ToolHost::new().run(ui, &pass, &mut TippingTool);
        }
    });
    shown.output.into()
}

/// The canvas menu asked for with Shift+F10: the line under the canvas says how the tool host reported the request
/// ([`ContextRequest::source`](super::ContextRequest::source)).
#[story(Canvas, covers = [], drive = Drive::Custom(press_menu_key))]
fn context_menu_key(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let room = stage(ui);
    let pixels_per_point = ui.pixels_per_point();
    let asked = data.value("asked", String::from("Right-click the canvas or press the Menu key.")).clone();
    let mut source = None;
    let row = |ui: &mut Ui, _: &mut BodyCx<'_>| {
        Text::new(asked.clone()).show(ui);
    };
    let stage = Stack::new().fill().footer(row).footer_gap(Gap::Stack).show_at(ui, room, |ui, stack| {
        let view = StoryDocument::STANDARD.view(stack.rect, pixels_per_point, 100.0);
        Canvas::new("Document")
            .view(view)
            .show_at(ui, stack.rect, |ui, body| {
                show_document(ui, body.rect, &StoryDocument::STANDARD);
                if let Some(pass) = tool_pass(&body.env) {
                    source = ToolHost::new().run(ui, &pass, &mut NoTool).context.map(|request| request.source).or(source);
                }
            })
            .output
    });
    let words = match source {
        Some(ContextSource::Key) => Some("The menu was asked for with a key: it opens at the view's centre."),
        Some(ContextSource::Pointer) => Some("The menu was asked for with the pointer: it opens where the pointer is."),
        None => None,
    };
    if let Some(words) = words {
        *data.value("asked", String::new()) = words.to_owned();
    }
    let Some(canvas) = stage.inner else { return StoryOutput::default() };
    canvas.into()
}
