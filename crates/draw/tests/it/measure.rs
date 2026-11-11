//! The Ruler: its line model, the options bar, the handles and the tool, and the distances between boxes.

use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{Axis, CancelReason, CanvasView, EditEvent};
use craft_draw::ui::measure::{RulerLine, RulerOverlay, RulerTool, box_distances};
use craft_ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use std::cell::RefCell;
use std::rc::Rc;

/// Boxes apart on one axis give one gap, overlapping boxes the distances between like edges, diagonal boxes a line
/// through the first box's centre with dashed extensions of the second box's edges, a box inside another four
/// distances; zero and non-finite distances are dropped; labels come from the closure with the axis.
#[trial]
fn apart_overlapping_diagonal_and_inside() {
    let run = |a: Rect, b: Rect| {
        let (lines, measures) = box_distances(a, b, |len, axis| format!("{len}{}", if axis == Axis::X { "x" } else { "y" }));
        assert!(lines.iter().all(|l| l.extension));
        let measures: Vec<_> = measures.into_iter().map(|m| (m.a, m.b, m.label.unwrap_or_default())).collect();
        (measures, lines.into_iter().map(|l| (l.a, l.b)).collect::<Vec<_>>())
    };
    let p = Point::new;
    let (m, e) = run(Rect::new(0.0, 0.0, 40.0, 40.0), Rect::new(100.0, 20.0, 150.0, 80.0));
    assert_eq!(
        m,
        vec![
            (p(40.0, 30.0), p(100.0, 30.0), "60x".into()),
            (p(20.0, 0.0), p(20.0, 20.0), "20y".into()),
            (p(20.0, 40.0), p(20.0, 80.0), "40y".into())
        ]
    );
    assert_eq!(e, vec![(p(100.0, 20.0), p(20.0, 20.0)), (p(100.0, 80.0), p(20.0, 80.0))]);

    let (m, e) = run(Rect::new(100.0, 0.0, 140.0, 20.0), Rect::new(20.0, 70.0, 70.0, 90.0));
    assert_eq!(m, vec![(p(70.0, 10.0), p(100.0, 10.0), "30x".into()), (p(120.0, 20.0), p(120.0, 70.0), "50y".into())]);
    assert_eq!(e, vec![(p(70.0, 70.0), p(70.0, 10.0)), (p(70.0, 70.0), p(120.0, 70.0))]);

    let (m, e) = run(Rect::new(40.0, 40.0, 90.0, 80.0), Rect::new(0.0, 0.0, 400.0, 300.0));
    assert_eq!(m.len(), 4);
    assert_eq!((m[0].0, m[0].1), (p(0.0, 60.0), p(40.0, 60.0)));
    assert_eq!((m[3].0, m[3].1), (p(65.0, 80.0), p(65.0, 300.0)));
    assert!(e.is_empty());

    let canvas = Rect::new(0.0, 0.0, 400.0, 300.0);
    assert!(run(canvas, canvas).0.is_empty());
    assert!(run(Rect::new(f64::NAN, 0.0, 1.0, 1.0), canvas).0.is_empty());
}

/// The tool for the fixture: it builds a pass over the shared line for every call and applies the events the pass
/// reported, as the app would.
struct Driven {
    tool: RulerTool,
    line: Rc<RefCell<Option<RulerLine>>>,
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Driven {
    fn sync(&mut self) {
        for event in self.tool.take_events() {
            let (phase, value) = match &event {
                EditEvent::Begin { .. } => ("begin", None),
                EditEvent::Update { value, .. } => ("update", Some(*value)),
                EditEvent::Commit { after, .. } => ("commit", Some(*after)),
                EditEvent::Set { after, .. } => ("set", Some(*after)),
                EditEvent::Cancel { before, .. } => (
                    "cancel",
                    before.as_ref().and_then(|b| match b {
                        craft_behavior::Mixed::Same(v) => Some(*v),
                        _ => None,
                    }),
                ),
                _ => ("other", None),
            };
            if let Some(value) = value {
                *self.line.borrow_mut() = value;
            }
            self.log.borrow_mut().push(phase);
        }
    }

    fn with_pass<R>(&mut self, call: impl FnOnce(&mut dyn CanvasTool) -> R) -> R {
        let line = *self.line.borrow();
        let result = call(&mut self.tool.pass(line));
        self.sync();
        result
    }
}

impl CanvasTool for Driven {
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        self.with_pass(|pass| pass.press(pointer, cx))
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with_pass(|pass| pass.drag(pointer, cx));
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.with_pass(|pass| pass.release(pointer, cx));
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        self.with_pass(|pass| pass.cancel(reason, cx));
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        self.with_pass(|pass| pass.key(key, cx))
    }

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Ruler Line... command")
    }
}

struct Shown {
    fixture: ToolFixture<Driven>,
}

fn shown(start: Option<RulerLine>) -> Shown {
    let line = Rc::new(RefCell::new(start));
    let log = Rc::new(RefCell::new(Vec::new()));
    let driven = Driven { tool: RulerTool::new(), line: Rc::clone(&line), log: Rc::clone(&log) };
    let body_line = Rc::clone(&line);
    let body_log = Rc::clone(&log);
    let fixture = ToolFixture::with_view(
        "Ruler",
        driven,
        |room| CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0),
        move |ui, body| {
            let mut line = *body_line.borrow();
            let out = RulerOverlay::new("Ruler", Bind::bound(&mut line)).show_at(ui, body.rect);
            body_log.borrow_mut().extend(out.events.iter().map(|event| match event {
                EditEvent::Begin { .. } => "begin",
                EditEvent::Update { .. } => "update",
                EditEvent::Commit { .. } => "commit",
                EditEvent::Set { .. } => "set",
                _ => "other",
            }));
            *body_line.borrow_mut() = line;
        },
    );
    Shown { fixture }
}

/// The tool owes the canvas a command form and a clean gesture: one begin, one commit, and Escape cancels.
#[trial]
fn the_ruler_tool_passes_the_tool_checks() {
    shown(None).fixture.check();
}
