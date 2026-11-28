//! Stories of the annotation toolbar and overlay, and the tool drawing a rectangle.

use std::sync::{Arc, Mutex};

use craft_behavior::CanvasView;
use craft_ui::canvas::{Canvas, ToolHost, tool_pass};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, pointer, story};
use egui::Ui;
use kurbo::{Point, Rect, Vec2};

use super::{AnnotateCommand, AnnotateIcons, AnnotateTool, AnnotationOverlay, AnnotationToolbar, BodyPaint};
use crate::annotate::{Annotation, AnnotationEdit, AnnotationGroup, AnnotationId, AnnotationKind, AnnotationTool, HandleRule};

fn annotation(id: u64, kind: AnnotationKind, rect: Rect) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        page: 0,
        kind,
        rect,
        points: Vec::new(),
        strokes: Vec::new(),
        color: [0xE0, 0x30, 0x30],
        opacity: 1.0,
        stroke_width: 2.0,
        text: String::new(),
        locked: false,
        aspect: None,
        handle_rule: HandleRule::Free,
    }
}

/// One annotation of each family: a box, a pointwise line, ink, a text markup and a note.
fn samples() -> Vec<Annotation> {
    let mut line = annotation(2, AnnotationKind::Arrow, Rect::new(20.0, 110.0, 140.0, 150.0));
    line.points = vec![Point::new(20.0, 110.0), Point::new(140.0, 150.0)];
    let mut ink = annotation(3, AnnotationKind::Ink, Rect::new(160.0, 100.0, 260.0, 160.0));
    ink.strokes = vec![vec![Point::new(160.0, 150.0), Point::new(200.0, 100.0), Point::new(260.0, 160.0)]];
    let mut markup = annotation(4, AnnotationKind::Highlight, Rect::new(20.0, 20.0, 140.0, 40.0));
    markup.points = vec![Point::new(20.0, 20.0), Point::new(140.0, 20.0), Point::new(140.0, 40.0), Point::new(20.0, 40.0)];
    markup.color = [0xFF, 0xE0, 0x30];
    markup.opacity = 0.4;
    let mut note = annotation(5, AnnotationKind::Note, Rect::new(180.0, 30.0, 196.0, 46.0));
    note.points = vec![Point::new(180.0, 30.0)];
    vec![annotation(1, AnnotationKind::Rectangle, Rect::new(20.0, 60.0, 140.0, 100.0)), line, ink, markup, note]
}

/// A callout whose knee sits left of its box, with its text.
fn callout(id: u64) -> Annotation {
    let mut callout = annotation(id, AnnotationKind::Callout, Rect::new(160.0, 60.0, 260.0, 100.0));
    callout.points = vec![Point::new(60.0, 160.0), Point::new(140.0, 80.0)];
    callout.text = "Check this figure".to_owned();
    callout
}

/// Shows `body` over a canvas whose view puts page (0, 0) at the canvas's top left at zoom 1.
fn on_canvas(ui: &mut Ui, mut body: impl FnMut(&mut Ui, egui::Rect)) -> egui::Rect {
    let room = ui.available_rect_before_wrap();
    let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
    Canvas::new("Page").view(view).show(ui, |ui, cx| body(ui, cx.rect));
    room
}

#[story(AnnotationToolbar<'static>, covers = [], drive = Drive::Never)]
fn toolbar(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut tool = AnnotateTool::new(AnnotationTool::Select, 0);
    AnnotationToolbar::new(&mut tool).show(ui);
    StoryOutput::default()
}

/// A group's button shows the tool last used from its flyout.
#[story(AnnotationToolbar<'static>, covers = [], drive = Drive::Never)]
fn toolbar_flyout_tool(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut tool = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Oval), 0);
    AnnotationToolbar::new(&mut tool).show(ui);
    StoryOutput::default()
}

/// Highlight is active: the colour well shows the highlight colour and the width field is off.
#[story(AnnotationToolbar<'static>, covers = [], drive = Drive::Never)]
fn toolbar_highlight(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut tool = AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Highlight), 0);
    AnnotationToolbar::new(&mut tool).show(ui);
    StoryOutput::default()
}

/// The glyphs an app supplies for some tools, in place of the framework's.
fn app_icon_set() -> AnnotateIcons {
    AnnotateIcons::new()
        .with(AnnotateCommand::Note, "text")
        .with(AnnotateCommand::Highlight, "pen_tool")
        .with(AnnotateCommand::FreehandHighlight, "pencil")
        .with(AnnotateCommand::Ink, "brush")
}

/// Gives every toolbar command its state with the app's icons, as an app's `FrameInput.commands` does.
fn supply_app_icons(fixture: &mut Fixture) {
    let icons = app_icon_set();
    let tools = std::iter::once(AnnotationTool::Select).chain(AnnotationGroup::ALL.iter().flat_map(|group| group.tools().iter().copied()));
    for command in tools.map(AnnotateCommand::of) {
        fixture.command_state(command, command.state(&icons));
    }
    fixture.frame(Vec::new());
}

/// The toolbar with the app's glyphs for Note, Highlight, Freehand-Highlight and Draw, beside the framework's for the
/// rest, so the two sets can be compared.

#[story(AnnotationToolbar<'static>, covers = [], drive = Drive::Custom(supply_app_icons))]
fn app_icons(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut tool = AnnotateTool::new(AnnotationTool::Select, 0);
    AnnotationToolbar::new(&mut tool).show(ui);
    StoryOutput::default()
}

/// A box, a line, ink, a text markup and a note; the box, the line and the markup are selected.
#[story(AnnotationOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn overlay(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let annotations = samples();
    let selected = [AnnotationId(1), AnnotationId(2), AnnotationId(4)];
    on_canvas(ui, |ui, at| {
        AnnotationOverlay::new("Annotations", &annotations, &selected).show_at(ui, at);
    });
    StoryOutput::default()
}

/// A rectangle and then a callout selected: the callout, the primary, shows its box handles, its tip and knee handles
/// and its leader; the rectangle shows its frame only.
#[story(AnnotationOverlay<'static>, covers = [], drive = Drive::Never)]
fn callout_selected(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let annotations = vec![annotation(1, AnnotationKind::Rectangle, Rect::new(20.0, 20.0, 100.0, 50.0)), callout(6)];
    let selected = [AnnotationId(1), AnnotationId(6)];
    on_canvas(ui, |ui, at| {
        AnnotationOverlay::new("Annotations", &annotations, &selected).show_at(ui, at);
    });
    StoryOutput::default()
}

/// A freehand highlight: broad translucent strokes.
#[story(AnnotationOverlay<'static>, covers = [], drive = Drive::Never)]
fn freehand_highlight(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut highlight = annotation(1, AnnotationKind::FreehandHighlight, Rect::new(20.0, 20.0, 220.0, 80.0));
    highlight.strokes = vec![vec![Point::new(20.0, 40.0), Point::new(90.0, 30.0), Point::new(160.0, 50.0), Point::new(220.0, 40.0)]];
    highlight.color = [0xFF, 0xE0, 0x30];
    highlight.opacity = 0.4;
    highlight.stroke_width = 12.0;
    let annotations = [highlight];
    on_canvas(ui, |ui, at| {
        AnnotationOverlay::new("Annotations", &annotations, &[]).show_at(ui, at);
    });
    StoryOutput::default()
}

/// The host paints every body with the page: the overlay draws only the frames of the selected markup, line and box,
/// and the handles of the box, the primary (the last selected).
#[story(AnnotationOverlay<'static>, covers = [], drive = Drive::Never)]
fn chrome_only(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let annotations = samples();
    let selected = [AnnotationId(4), AnnotationId(2), AnnotationId(1)];
    on_canvas(ui, |ui, at| {
        AnnotationOverlay::new("Annotations", &annotations, &selected).bodies(BodyPaint::Only(&[])).show_at(ui, at);
    });
    StoryOutput::default()
}

/// How far the in-flight story's move has taken the annotations, in document points.
const IN_FLIGHT_MOVE: Vec2 = Vec2::new(30.0, 20.0);

/// The box and the line part way through a move: both are painted where the move takes them.
#[story(AnnotationOverlay<'static>, covers = [], drive = Drive::Never)]
fn in_flight(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let annotations = samples();
    let selected = [AnnotationId(1), AnnotationId(2)];
    let edit = AnnotationEdit::Move { ids: selected.to_vec(), delta: IN_FLIGHT_MOVE };
    on_canvas(ui, |ui, at| {
        AnnotationOverlay::new("Annotations", &annotations, &selected).live(&edit).show_at(ui, at);
    });
    StoryOutput::default()
}

/// A commented highlight shows a marker with a leader to its text; a plain one shows none.
#[story(AnnotationOverlay<'static>, covers = [], drive = Drive::Never)]
fn marker(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let highlight = |id: u64, r: Rect, text: &str| {
        let mut a = annotation(id, AnnotationKind::Highlight, r);
        a.points = vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        a.color = [0xFF, 0xE0, 0x30];
        a.opacity = 0.4;
        a.text = text.to_owned();
        a
    };
    let annotations = [highlight(1, Rect::new(20.0, 60.0, 160.0, 76.0), "Is this right?"), highlight(2, Rect::new(20.0, 120.0, 160.0, 136.0), "")];
    on_canvas(ui, |ui, at| {
        AnnotationOverlay::new("Annotations", &annotations, &[]).show_at(ui, at);
    });
    StoryOutput::default()
}

/// A selected line whose points the host keeps: its frame, and no handle.
#[story(AnnotationOverlay<'static>, covers = [], drive = Drive::Never)]
fn fixed_points(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut line = annotation(1, AnnotationKind::Line, Rect::new(20.0, 60.0, 200.0, 60.0));
    line.points = vec![Point::new(20.0, 60.0), Point::new(200.0, 60.0)];
    line.handle_rule = HandleRule::FixedPoints;
    let annotations = [line];
    let selected = [AnnotationId(1)];
    on_canvas(ui, |ui, at| {
        AnnotationOverlay::new("Annotations", &annotations, &selected).show_at(ui, at);
    });
    StoryOutput::default()
}

/// A selected callout the host cannot resize: its frame and leader, and no handle.
#[story(AnnotationOverlay<'static>, covers = [], drive = Drive::Never)]
fn no_resize(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut callout = callout(1);
    callout.handle_rule = HandleRule::NoResize;
    let annotations = [callout];
    let selected = [AnnotationId(1)];
    on_canvas(ui, |ui, at| {
        AnnotationOverlay::new("Annotations", &annotations, &selected).show_at(ui, at);
    });
    StoryOutput::default()
}

/// The tool, shared between passes.
#[derive(Clone)]
struct Shared(Arc<Mutex<AnnotateTool>>);

impl PartialEq for Shared {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Drags the Rectangle tool from the middle of the canvas.
fn drag_rectangle(fixture: &mut Fixture) {
    let Some(canvas) = fixture.by_role(craft_ui::contract::Role::CANVAS).into_iter().next() else { return };
    let from = canvas.center();
    // A quarter of the canvas, reached in four steps.
    let by = canvas.rect.size() / 4.0;
    fixture.frame(vec![pointer::moved(from)]);
    fixture.frame(vec![pointer::press(from)]);
    for step in 1..=4 {
        fixture.frame(vec![pointer::moved(from + by * (step as f32 / 4.0))]);
    }
}

/// The Rectangle tool part way through a drag: its draft outline follows the pointer.
#[story(AnnotationOverlay<'static>, covers = [], drive = Drive::Custom(drag_rectangle))]
fn tool_drawing_a_rectangle(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shared = data.value("tool", Shared(Arc::new(Mutex::new(AnnotateTool::new(AnnotationTool::Draw(AnnotationKind::Rectangle), 0)))));
    let annotations = samples();
    let room = ui.available_rect_before_wrap();
    let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
    let _ = Canvas::new("Page").view(view).show(ui, |ui, cx| {
        AnnotationOverlay::new("Annotations", &annotations, &[]).show_at(ui, cx.rect);
        if let (Some(pass), Ok(mut tool)) = (tool_pass(&cx.env), shared.0.lock()) {
            let _ = ToolHost::new().run(ui, &pass, &mut *tool);
        }
    });
    StoryOutput::default()
}
