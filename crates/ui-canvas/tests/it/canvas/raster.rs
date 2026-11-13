//! `RasterDisplay` and `Thumbnails`: what is read again, the detail plan, the fit maths and the request rules.

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::{Point, Rect as Rect64};
use craft_behavior::{CanvasView, PixelSize};
use craft_ui::canvas::{AppPaint, Canvas, Extent, PageFrame, Pasteboard, PixelRect, RasterDisplay, RasterSource};
use craft_ui::canvas_controls::PixelGrid;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::primitives::{Checker, Revision};
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, Mesh, PaintCallback, Pos2, Rect, Shape, TextureId, Vec2};
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
/// its shadow, a display of `document` with the checkerboard and the edge, and the pixel grid over it. While `painted`
/// is set the display is a painted one: the app's paint adds a GPU paint callback, as an app with its own compositor
/// does, and counts its calls in `paints`.
fn document_canvas(document: Rc<Document>, painted: Rc<Cell<bool>>, paints: Rc<Cell<usize>>) -> Fixture {
    let size = document.size;
    let center = Point::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, 8.0);
    let pages = [PageFrame::new(Rect64::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)))];
    let callback: Arc<dyn std::any::Any + Send + Sync> = Arc::new(());
    let mut fixture = Fixture::new(move |ui| {
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            let _ = Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect);
            let display = if painted.get() {
                let paint = AppPaint::new(|painter, view: CanvasView| {
                    paints.set(paints.get() + 1);
                    painter.painter.add(Shape::Callback(PaintCallback { rect: view.rect, callback: Arc::clone(&callback) }));
                });
                RasterDisplay::painted("Picture", size, paint)
            } else {
                RasterDisplay::new("Picture", &*document)
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
    let painted = Rc::new(Cell::new(true));
    let mut fixture = document_canvas(Document::new(40, 30), Rc::clone(&painted), Rc::new(Cell::new(0)));
    let from_app = fixture.frame_shapes();
    painted.set(false);
    fixture.settle();
    let from_source = fixture.frame_shapes();
    let callbacks: Vec<usize> = from_app.iter().enumerate().filter_map(|(at, shape)| matches!(shape, Shape::Callback(_)).then_some(at)).collect();
    let [at] = callbacks[..] else { panic!("the app's paint is in the frame once: {callbacks:?}") };
    let (under, over) = (&from_app[..at], &from_app[at + 1..]);
    assert!(!under.is_empty() && !over.is_empty(), "the shadow and the checkerboard paint under the picture, the edge and the grid over it");
    // The document fits one texture at one texel per pixel, so the source's picture is one textured quad.
    let same_under = from_source.iter().zip(under).take_while(|(a, b)| a == b).count();
    let same_over = from_source.iter().rev().zip(over.iter().rev()).take_while(|(a, b)| a == b).count();
    assert!(
        from_source.len() == under.len() + 1 + over.len() && same_under == under.len() && same_over == over.len(),
        "the app's picture takes the place of the source's texture between the same shapes: {same_under} of {} under, {same_over} of {} \
         over, {} shapes from the source",
        under.len(),
        over.len(),
        from_source.len()
    );
}

#[trial]
fn a_painted_display_paints_once_a_frame() {
    let paints = Rc::new(Cell::new(0));
    let mut fixture = document_canvas(Document::new(40, 30), Rc::new(Cell::new(true)), Rc::clone(&paints));
    for _ in 0..3 {
        paints.set(0);
        fixture.frame(Vec::new());
        assert_eq!(paints.get(), 1, "one paint in a frame of {} passes", fixture.passes());
    }
}

/// A canvas looking at `center` at `zoom` that shows a display of `document` with the checkerboard, the edge,
/// `extent` and, if given, mapped through `frame`: a source display, or, while `painted` is set, a painted one whose
/// paint adds a GPU paint callback and records the view of each call in `paints`.
fn extent_canvas(
    document: Rc<Document>,
    center: Point,
    zoom: f64,
    extent: Rc<Cell<Extent>>,
    frame: Option<Homography>,
    painted: bool,
    paints: Rc<RefCell<Vec<CanvasView>>>,
) -> Fixture {
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, zoom);
    let callback: Arc<dyn std::any::Any + Send + Sync> = Arc::new(());
    let mut fixture = Fixture::new(move |ui| {
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            let display = if painted {
                let paint = AppPaint::new(|painter, view: CanvasView| {
                    paints.borrow_mut().push(view);
                    painter.painter.add(Shape::Callback(PaintCallback { rect: view.rect, callback: Arc::clone(&callback) }));
                });
                RasterDisplay::painted("Picture", document.size, paint)
            } else {
                RasterDisplay::new("Picture", &*document)
            };
            let display = match frame {
                Some(frame) => display.mapped(frame, 4),
                None => display,
            };
            display.checkerboard(Checker::new()).outlined().extent(extent.get()).show_at(ui, body.rect);
        });
    });
    fixture.room(CANVAS);
    fixture.settle();
    fixture
}

/// The screen bounds of each textured mesh in `shapes`, in paint order: the checkerboard, then a source's pictures.
fn textured(shapes: &[Shape]) -> Vec<&Mesh> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() => Some(&**mesh),
            _ => None,
        })
        .collect()
}

fn doc_rect(view: &CanvasView, r: Rect64) -> Rect {
    Rect::from_points(&view.quad(r))
}

fn near(a: Rect, b: Rect) -> bool {
    (a.min - b.min).length() < 0.01 && (a.max - b.max).length() < 0.01
}

/// One frame of a display of `document` with `extent`, either picture: where each picture of the document went on
/// screen (a source's textured quads, or the document through each view the app's paint was given), the
/// checkerboard's bounds, and the bounds of everything painted after the pictures (the edge).
/// The pictures a frame places, the checkerboard's areas (the meshes drawn with the first textured mesh's texture), and
/// the bounds of what is drawn after the last picture.
fn pictures(
    document: &Rc<Document>,
    center: Point,
    zoom: f64,
    extent: Extent,
    frame: Option<Homography>,
    painted: bool,
) -> (Vec<Rect>, Vec<Rect>, Rect) {
    let paints = Rc::new(RefCell::new(Vec::new()));
    let mut fixture = extent_canvas(Rc::clone(document), center, zoom, Rc::new(Cell::new(extent)), frame, painted, Rc::clone(&paints));
    paints.borrow_mut().clear();
    let shapes = fixture.frame_shapes();
    let meshes = textured(&shapes);
    let (checker, sources): (Vec<&Mesh>, Vec<&Mesh>) = meshes.iter().copied().partition(|mesh| mesh.texture_id == meshes[0].texture_id);
    let checker: Vec<Rect> = checker.iter().map(|mesh| mesh.calc_bounds()).collect();
    let whole = Rect64::new(0.0, 0.0, f64::from(document.size.width), f64::from(document.size.height));
    let placed: Vec<Rect> = if painted {
        paints.borrow().iter().map(|view| doc_rect(view, whole)).collect()
    } else {
        sources.iter().map(|mesh| mesh.calc_bounds()).collect()
    };
    let last_picture = shapes
        .iter()
        .rposition(|shape| matches!(shape, Shape::Callback(_)) || matches!(shape, Shape::Mesh(mesh) if mesh.texture_id != TextureId::default()))
        .unwrap_or(0);
    let after = shapes[last_picture + 1..].iter().fold(Rect::NOTHING, |all, shape| all.union(shape.visual_bounding_rect()));
    (placed, checker, after)
}

#[trial]
fn tiled_copies_fill_the_canvas_a_document_apart() {
    // A 100 x 70 document at 50 percent: each copy is 50 x 35 points, and no copy's edge falls on the canvas's.
    let document = Document::new(100, 70);
    let center = Point::new(50.0, 35.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, 0.5);
    let canvas = view.rect;
    let at =
        |i: i32, j: i32| doc_rect(&view, Rect64::new(f64::from(i) * 100.0, f64::from(j) * 70.0, f64::from(i + 1) * 100.0, f64::from(j + 1) * 70.0));
    let in_view: Vec<Rect> = (-40..40).flat_map(|j| (-40..40).map(move |i| (i, j))).map(|(i, j)| at(i, j)).filter(|r| r.intersects(canvas)).collect();
    for painted in [false, true] {
        let (placed, checker, edge) = pictures(&document, center, 0.5, Extent::Tiled, None, painted);
        assert_eq!(placed.len(), in_view.len(), "painted {painted}: one picture for each document-sized tile the canvas shows");
        assert!(in_view.iter().all(|tile| placed.iter().any(|r| near(*r, *tile))), "painted {painted}: every tile in view has its copy");
        assert!(near(placed[0], at(0, 0)), "painted {painted}: the document draws first, in its place");
        let all = in_view.iter().fold(Rect::NOTHING, |all, r| all.union(*r));
        assert!(matches!(checker[..], [c] if near(c, all)), "painted {painted}: the checkerboard lies under the copies: {checker:?} vs {all:?}");
        assert!(
            edge.expand(-3.0).contains_rect(at(0, 0).expand(-6.0)) && at(0, 0).expand(3.0).contains_rect(edge),
            "painted {painted}: the edge stays on the document: {edge:?}"
        );

        let (filled, _, _) = pictures(&document, center, 20.0, Extent::Tiled, None, painted);
        assert_eq!(filled.len(), 1, "painted {painted}: a document filling the canvas has no copies");

        // A one-pixel document far out would need thousands of copies; the run stops at 31 a side, centred on the view.
        let (capped, _, _) = pictures(&Document::new(1, 1), Point::new(0.5, 0.5), 0.5, Extent::Tiled, None, painted);
        assert_eq!(capped.len(), 31 * 31, "painted {painted}: copies are capped");
        let middle = capped.iter().fold(Rect::NOTHING, |all, r| all.union(*r)).center();
        assert!((middle - canvas.center()).length() < 1.0, "painted {painted}: the capped run is centred on the view: {middle:?}");

        // Panned away from the document, the capped run leaves it out: the checkerboard lies under the run and under
        // the document, and nowhere between them.
        let tiny = Document::new(1, 1);
        let center = Point::new(500.5, 0.5);
        let panned = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, 0.5);
        let (capped, checker, _) = pictures(&tiny, center, 0.5, Extent::Tiled, None, painted);
        assert_eq!(capped.len(), 31 * 31 + 1, "painted {painted}: the capped run and the document");
        let run = doc_rect(&panned, Rect64::new(485.0, -15.0, 516.0, 16.0));
        let page = doc_rect(&panned, Rect64::new(0.0, 0.0, 1.0, 1.0));
        assert!(near(capped[0], page), "painted {painted}: the document draws first, outside the run: {:?}", capped[0]);
        assert!(
            matches!(checker[..], [a, b] if near(a, run) && near(b, page)),
            "painted {painted}: the checkerboard covers the run and the document only: {checker:?}"
        );
    }
}

#[trial]
fn a_mapped_picture_tiles_its_area_a_whole_area_apart() {
    // A 100 x 70 document whose picture covers 140 x 90 from (-20, -10) (a preview that grew the image), at 50 percent.
    let document = Document::new(100, 70);
    let area = Rect64::new(-20.0, -10.0, 120.0, 80.0);
    let corners = [Point::new(area.x0, area.y0), Point::new(area.x1, area.y0), Point::new(area.x1, area.y1), Point::new(area.x0, area.y1)];
    let frame = Homography::unit_to_quad(corners).expect("a rectangle");
    let center = Point::new(50.0, 35.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, 0.5);
    let at = |i: i32, j: i32| doc_rect(&view, area + craft_behavior::kurbo::Vec2::new(f64::from(i) * 140.0, f64::from(j) * 90.0));
    let in_view: Vec<Rect> =
        (-40..40).flat_map(|j| (-40..40).map(move |i| (i, j))).map(|(i, j)| at(i, j)).filter(|r| r.intersects(view.rect)).collect();
    for painted in [false, true] {
        let (placed, checker, edge) = pictures(&document, center, 0.5, Extent::Tiled, Some(frame), painted);
        assert_eq!(placed.len(), in_view.len(), "painted {painted}: one picture for each area-sized tile the canvas shows");
        let all = in_view.iter().fold(Rect::NOTHING, |all, r| all.union(*r));
        assert!(matches!(checker[..], [c] if near(c, all)), "painted {painted}: the checkerboard lies under the copies: {checker:?} vs {all:?}");
        assert!(
            edge.expand(-3.0).contains_rect(at(0, 0).expand(-6.0)) && at(0, 0).expand(3.0).contains_rect(edge),
            "painted {painted}: the edge stays on the picture's area: {edge:?}"
        );
        if !painted {
            assert!(in_view.iter().all(|tile| placed.iter().any(|r| near(*r, *tile))), "every tile in view has its copy");
            assert!(near(placed[0], at(0, 0)), "the picture draws first, in its area");
        }
    }
}

#[trial]
fn a_region_past_the_document_draws_out_to_the_region() {
    let document = Document::new(100, 70);
    let region = PixelRect::new(-60, -30, 200, 120);
    let center = Point::new(70.0, 45.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, 2.0);
    let on_screen = |r: PixelRect| doc_rect(&view, Rect64::new(f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)));
    let page = on_screen(PixelRect::new(0, 0, 100, 70));
    for painted in [false, true] {
        document.reads.borrow_mut().clear();
        let (placed, checker, edge) = pictures(&document, center, 2.0, Extent::Region(region), None, painted);
        assert!(matches!(checker[..], [c] if near(c, on_screen(region))), "painted {painted}: the checkerboard covers the region: {checker:?}");
        assert!(
            edge.expand(-3.0).contains_rect(page.expand(-6.0)) && page.expand(3.0).contains_rect(edge),
            "painted {painted}: the edge stays on the document: {edge:?}"
        );
        if painted {
            assert_eq!(placed.len(), 1, "the app paints once, through the view on screen");
            assert!(document.reads.borrow().is_empty(), "a painted display reads nothing");
        } else {
            assert_eq!(placed.len(), 1, "one picture");
            assert!(near(placed[0], on_screen(region)), "the source's picture spans the region: {:?}", placed[0]);
            assert!(document.reads.borrow().contains(&(region, 1)), "the source is read past the document: {:?}", document.reads.borrow());
        }
    }

    // The same size of region moved: read again, never shown from the texture of the old place.
    let extent = Rc::new(Cell::new(Extent::Region(region)));
    let mut fixture = extent_canvas(Rc::clone(&document), center, 2.0, Rc::clone(&extent), None, false, Rc::new(RefCell::new(Vec::new())));
    let moved = PixelRect::new(-50, -30, 210, 120);
    document.reads.borrow_mut().clear();
    extent.set(Extent::Region(moved));
    fixture.frame(Vec::new());
    assert!(document.reads.borrow().contains(&(moved, 1)), "a moved region is read: {:?}", document.reads.borrow());
}

#[trial]
fn a_picture_whose_area_is_off_the_document_has_its_checkerboard_and_edge_there() {
    // A 100 x 70 document; its picture turned a quarter about the document's centre (70 x 100), then moved past the
    // document's bottom right corner.
    let document = Document::new(100, 70);
    let center = Point::new(50.0, 35.0);
    let view = CanvasView::new(Rect::from_min_size(Pos2::ZERO, CANVAS), center, 2.0);
    let page = doc_rect(&view, Rect64::new(0.0, 0.0, 100.0, 70.0));
    for area in [Rect64::new(15.0, -15.0, 85.0, 85.0), Rect64::new(50.0, 35.0, 150.0, 105.0)] {
        let corners = [Point::new(area.x0, area.y0), Point::new(area.x1, area.y0), Point::new(area.x1, area.y1), Point::new(area.x0, area.y1)];
        let frame = Homography::unit_to_quad(corners).expect("a rectangle");
        let on_screen = doc_rect(&view, area);
        for painted in [false, true] {
            let (placed, checker, edge) = pictures(&document, center, 2.0, Extent::Document, Some(frame), painted);
            assert!(matches!(checker[..], [c] if near(c, on_screen)), "painted {painted}: the checkerboard lies under {area:?}: {checker:?}");
            assert!(
                edge.expand(-3.0).contains_rect(on_screen.expand(-6.0)) && on_screen.expand(3.0).contains_rect(edge),
                "painted {painted}: the edge lies around {area:?}: {edge:?}"
            );
            assert_eq!(placed.len(), 1, "painted {painted}: one picture");
            let picture = if painted { page } else { on_screen };
            assert!(
                near(placed[0], picture),
                "painted {painted}: the app paints through the view as given, a source fills the area: {:?}",
                placed[0]
            );
        }
    }
}
