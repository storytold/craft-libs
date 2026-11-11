//! `RasterDisplay` and `Thumbnails`: what is read again, the detail plan, the fit maths and the request rules.

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, PixelSize};
use craft_ui::canvas::{Canvas, PixelRect, RasterDisplay, RasterSource};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::primitives::{Checker, Revision};
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

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
