//! Stories of the annotation toolbar and overlay, and the tool drawing a rectangle.

use std::sync::{Arc, Mutex};

use craft_behavior::CanvasView;
use craft_ui::canvas::{Canvas, ToolHost};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, pointer, story};
use egui::Ui;
use kurbo::{Point, Rect};

use super::{AnnotateTool, AnnotationOverlay, AnnotationToolbar};
use crate::annotate::{Annotation, AnnotationId, AnnotationKind, AnnotationTool};

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
    let canvas = Canvas::new("Page").view(view).show(ui, |ui, cx| {
        AnnotationOverlay::new("Annotations", &annotations, &[]).show_at(ui, cx.rect);
    });
    if let Ok(mut tool) = shared.0.lock() {
        let _ = ToolHost::new().run(ui, &canvas.output, &mut *tool);
    }
    StoryOutput::default()
}
