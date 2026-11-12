//! `RasterDisplay` and `Thumbnails`: what is read again, the detail plan, the fit maths and the request rules.

use craft_behavior::kurbo::{Point, Rect as Rect64};
use craft_behavior::{CanvasView, PixelSize};
use craft_ui::canvas::{AppPaint, Canvas, PageFrame, Pasteboard, PixelRect, RasterDisplay, RasterSource};
use craft_ui::canvas_controls::PixelGrid;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::primitives::{Checker, Revision};
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, PaintCallback, Pos2, Rect, Shape, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// The canvas the display is shown in, in points.
const CANVAS: Vec2 = Vec2::new(800.0, 600.0);

/// A document that records every read and every change: the revision it reached and the region that changed
/// (`None` for a change it cannot place).
struct Document {
    size: PixelSize,
    revision: Cell<u64>,
    changes: RefCell<Vec<(u64, Option<PixelRect>)>>,
    reads: RefCell<Vec<(PixelRect, u32)>>,
}

impl Document {
    fn new(width: u32, height: u32) -> Rc<Self> {
        Rc::new(Document {
            size: PixelSize::new(width, height),
            revision: Cell::new(1),
            changes: RefCell::new(Vec::new()),
            reads: RefCell::new(Vec::new()),
        })
    }

    /// Moves to `revision`, with `damage` changed.
    fn change(&self, revision: u64, damage: Option<PixelRect>) {
        self.revision.set(revision);
        self.changes.borrow_mut().push((revision, damage));
    }
}

impl RasterSource for Document {
    fn size(&self) -> PixelSize {
        self.size
    }

    fn revision(&self) -> Revision {
        Revision::new(self.revision.get())
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        self.changes
            .borrow()
            .iter()
            .filter(|(revision, _)| *revision > since.get())
            .try_fold(PixelRect::default(), |all, (_, damage)| damage.map(|damage| all.union(&damage)))
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        self.reads.borrow_mut().push((region, factor));
        out.pixels.fill(Color32::from_gray(40));
    }
}

/// A display of `document` in a canvas looking at the document's centre at `zoom`.
struct Rig {
    fixture: Fixture,
    document: Rc<Document>,
    shown: Rc<Cell<bool>>,
}

fn rig(document: Rc<Document>, zoom: f64) -> Rig {
    let center = Point::new(f64::from(document.size.width) / 2.0, f64::from(document.size.height) / 2.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, zoom);
    let shown = Rc::new(Cell::new(true));
    let (shown_document, shown_now) = (Rc::clone(&document), Rc::clone(&shown));
    let mut fixture = Fixture::new(move |ui| {
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            if shown_now.get() {
                RasterDisplay::new("Picture", &*shown_document).checkerboard(Checker::new()).show_at(ui, body.rect);
            }
        });
    });
    fixture.room(CANVAS);
    fixture.settle();
    Rig { fixture, document, shown }
}

impl Rig {
    /// One frame, and the reads it made.
    fn frame(&mut self) -> Vec<(PixelRect, u32)> {
        self.document.reads.borrow_mut().clear();
        self.fixture.frame(Vec::new());
        self.document.reads.borrow().clone()
    }

    /// One frame after the document changed to `revision` with `damage`.
    fn change(&mut self, revision: u64, damage: Option<PixelRect>) -> Vec<(PixelRect, u32)> {
        self.document.change(revision, damage);
        self.frame()
    }
}

#[trial]
fn a_pass_the_display_skipped_loses_no_damage() {
    let mut rig = rig(Document::new(400, 300), 1.0);
    let (first, second) = (PixelRect::new(10, 20, 50, 60), PixelRect::new(100, 100, 120, 120));
    rig.shown.set(false);
    assert!(rig.change(2, Some(first)).is_empty(), "a display that is not shown reads nothing");
    rig.shown.set(true);
    let reads = rig.change(3, Some(second));
    let both = first.union(&second);
    assert!(!reads.is_empty() && reads.iter().all(|read| *read == (both, 1)), "the display reads both changes: {reads:?}");
}

#[trial]
fn the_checkerboard_paints_the_same_shapes_at_any_area() {
    let shapes = |document: Rc<Document>, zoom: f64| {
        let mut rig = rig(document, zoom);
        rig.frame();
        rig.fixture.shape_bounds().len()
    };
    let small = shapes(Document::new(40, 30), 1.0);
    let large = shapes(Document::new(400, 300), 4.0);
    assert_eq!(small, large, "a checkerboard over the whole canvas paints as many shapes as one over a corner of it");
}

/// A canvas at 800 percent whose body shows, as a document canvas does, the pasteboard with the document's page and
/// its shadow, a display of `document` with the checkerboard and the edge, and the pixel grid over it. A painted
/// display (`paints` given) gets the app's paint: a GPU paint callback, as an app with its own compositor adds, which
/// counts its calls in `paints`.
fn document_canvas(document: Rc<Document>, paints: Option<Rc<Cell<usize>>>) -> Fixture {
    let size = document.size;
    let center = Point::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, 8.0);
    let pages = [PageFrame::new(Rect64::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)))];
    let callback: Arc<dyn std::any::Any + Send + Sync> = Arc::new(());
    let mut fixture = Fixture::new(move |ui| {
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            let _ = Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect);
            let display = match &paints {
                Some(paints) => RasterDisplay::painted(
                    "Picture",
                    size,
                    AppPaint::new(|painter, view: CanvasView| {
                        paints.set(paints.get() + 1);
                        painter.painter.add(Shape::Callback(PaintCallback { rect: view.rect, callback: Arc::clone(&callback) }));
                    }),
                ),
                None => RasterDisplay::new("Picture", &*document),
            };
            display.checkerboard(Checker::new()).outlined().show_at(ui, body.rect);
            let _ = PixelGrid::new("Pixel grid", &*document).show_at(ui, body.rect);
        });
    });
    fixture.room(CANVAS);
    fixture.settle();
    fixture
}

#[trial]
fn a_painted_display_draws_the_same_surround_as_a_source_display() {
    let from_source = document_canvas(Document::new(40, 30), None).frame_shapes();
    let painted = document_canvas(Document::new(40, 30), Some(Rc::new(Cell::new(0)))).frame_shapes();
    let callbacks: Vec<usize> = painted.iter().enumerate().filter_map(|(at, shape)| matches!(shape, Shape::Callback(_)).then_some(at)).collect();
    let [at] = callbacks[..] else { panic!("the app's paint is in the frame once: {callbacks:?}") };
    let (under, over) = (&painted[..at], &painted[at + 1..]);
    assert!(!under.is_empty() && !over.is_empty(), "the shadow and the checkerboard paint under the picture, the edge and the grid over it");
    assert!(
        from_source.len() > under.len() + over.len() && from_source.starts_with(under) && from_source.ends_with(over),
        "the app's picture takes the place of the source's textures between the same shapes"
    );
}

#[trial]
fn a_painted_display_paints_once_a_frame() {
    let paints = Rc::new(Cell::new(0));
    let mut fixture = document_canvas(Document::new(40, 30), Some(Rc::clone(&paints)));
    for _ in 0..3 {
        paints.set(0);
        fixture.frame(Vec::new());
        assert_eq!(paints.get(), 1, "one paint in a frame of {} passes", fixture.passes());
    }
}
